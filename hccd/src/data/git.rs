//! 变更面板的数据：git status 加上增删行数。

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::model::FileChange;

/// 未跟踪文件超过这个大小就不数行数
const MAX_COUNT_BYTES: u64 = 1024 * 1024;

/// 不是 git 仓库或 git 出错时返回空列表
pub fn changed_files(cwd: &Path) -> Vec<FileChange> {
    try_changed_files(cwd).unwrap_or_default()
}

fn try_changed_files(cwd: &Path) -> Option<Vec<FileChange>> {
    let root = git(cwd, &["rev-parse", "--show-toplevel"])?;
    let root = Path::new(String::from_utf8_lossy(&root).trim()).to_path_buf();
    let status = git(cwd, &["status", "--porcelain=v1", "-z", "--untracked-files=all"])?;

    let mut stats: HashMap<String, (u32, u32)> = HashMap::new();
    if git(cwd, &["rev-parse", "--verify", "--quiet", "HEAD"]).is_some() {
        add_numstat(&mut stats, &git(cwd, &["diff", "--numstat", "-z", "HEAD"])?);
    } else {
        // 还没有提交：暂存区相对空树，加上工作区相对暂存区
        add_numstat(&mut stats, &git(cwd, &["diff", "--numstat", "-z", "--cached"])?);
        add_numstat(&mut stats, &git(cwd, &["diff", "--numstat", "-z"])?);
    }

    let mut out = parse_status(&status);
    for c in &mut out {
        if c.status == '?' {
            c.added = count_lines(&root.join(&c.path));
        } else if let Some(&(a, r)) = stats.get(&c.path) {
            c.added = a;
            c.removed = r;
        }
    }
    Some(out)
}

/// 成功时返回 stdout
fn git(cwd: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        // 只读操作，不去抢 index.lock
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status.success().then_some(out.stdout)
}

/// porcelain v1 -z：每条 "XY path\0"，重命名/复制后面再跟一个 "原路径\0"
fn parse_status(raw: &[u8]) -> Vec<FileChange> {
    let mut out = Vec::new();
    let mut fields = raw.split(|&b| b == 0).filter(|f| !f.is_empty());
    while let Some(f) = fields.next() {
        if f.len() < 4 {
            continue;
        }
        let (x, y) = (f[0] as char, f[1] as char);
        let path = String::from_utf8_lossy(&f[3..]).into_owned();
        if matches!(x, 'R' | 'C') || matches!(y, 'R' | 'C') {
            fields.next();
        }
        let status = if x == '?' {
            '?'
        } else if x == 'R' || y == 'R' {
            'R'
        } else if x == 'D' || y == 'D' {
            'D'
        } else if x == 'A' || x == 'C' {
            'A'
        } else {
            'M'
        };
        out.push(FileChange { status, path, added: 0, removed: 0 });
    }
    out
}

/// numstat -z：普通文件 "增\t删\tpath\0"，重命名 "增\t删\t\0原路径\0新路径\0"；二进制是 "-" 记 0
fn add_numstat(stats: &mut HashMap<String, (u32, u32)>, raw: &[u8]) {
    let mut fields = raw.split(|&b| b == 0);
    while let Some(f) = fields.next() {
        let s = String::from_utf8_lossy(f);
        let mut parts = s.splitn(3, '\t');
        let (Some(a), Some(r), Some(path)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let path = if path.is_empty() {
            fields.next();
            match fields.next() {
                Some(p) => String::from_utf8_lossy(p).into_owned(),
                None => continue,
            }
        } else {
            path.to_string()
        };
        let e = stats.entry(path).or_default();
        e.0 += a.parse().unwrap_or(0);
        e.1 += r.parse().unwrap_or(0);
    }
}

/// 文本文件的行数；二进制、过大或读不了记 0
fn count_lines(path: &Path) -> u32 {
    let Ok(meta) = fs::metadata(path) else { return 0 };
    if !meta.is_file() || meta.len() > MAX_COUNT_BYTES {
        return 0;
    }
    let mut buf = Vec::new();
    if fs::File::open(path).and_then(|mut f| f.read_to_end(&mut buf)).is_err() || buf.contains(&0) {
        return 0;
    }
    let n = buf.iter().filter(|&&b| b == b'\n').count();
    let tail = usize::from(buf.last().is_some_and(|&b| b != b'\n'));
    (n + tail) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("hccd-git-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn run(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null"])
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }

    fn find<'a>(list: &'a [FileChange], path: &str) -> &'a FileChange {
        list.iter().find(|c| c.path == path).unwrap_or_else(|| panic!("{path} not in {list:?}"))
    }

    #[test]
    fn not_a_repo() {
        let d = temp_dir();
        assert!(changed_files(&d).is_empty());
        assert!(changed_files(&d.join("missing")).is_empty());
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn no_head_yet() {
        let d = temp_dir();
        run(&d, &["init", "-q"]);
        fs::write(d.join("a.txt"), "1\n2\n").unwrap();
        run(&d, &["add", "a.txt"]);
        fs::write(d.join("a.txt"), "1\n2\n3\n").unwrap();
        fs::write(d.join("u.txt"), "x").unwrap();
        let list = changed_files(&d);
        assert_eq!(find(&list, "a.txt"), &FileChange { status: 'A', path: "a.txt".into(), added: 3, removed: 0 });
        assert_eq!(find(&list, "u.txt").added, 1);
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn statuses() {
        let d = temp_dir();
        run(&d, &["init", "-q"]);
        fs::write(d.join("m.txt"), "a\nb\nc\n").unwrap();
        fs::write(d.join("del.txt"), "x\ny\n").unwrap();
        fs::write(d.join("old name.txt"), "same\ncontent\nhere\n").unwrap();
        run(&d, &["add", "."]);
        run(&d, &["commit", "-q", "-m", "init"]);

        fs::write(d.join("m.txt"), "a\nB\nc\nd\n").unwrap();
        fs::remove_file(d.join("del.txt")).unwrap();
        run(&d, &["mv", "old name.txt", "new name.txt"]);
        fs::write(d.join("new.txt"), "n\n").unwrap();
        run(&d, &["add", "new.txt"]);
        fs::create_dir_all(d.join("sub/deep")).unwrap();
        fs::write(d.join("sub/deep/u.txt"), "1\n2\n3").unwrap();
        fs::write(d.join("bin.dat"), [0u8, 1, 2, b'\n']).unwrap();

        let list = changed_files(&d);
        assert_eq!(list.len(), 6, "{list:?}");
        assert_eq!(find(&list, "m.txt"), &FileChange { status: 'M', path: "m.txt".into(), added: 2, removed: 1 });
        assert_eq!(find(&list, "del.txt"), &FileChange { status: 'D', path: "del.txt".into(), added: 0, removed: 2 });
        assert_eq!(find(&list, "new name.txt").status, 'R');
        assert_eq!(find(&list, "new.txt"), &FileChange { status: 'A', path: "new.txt".into(), added: 1, removed: 0 });
        assert_eq!(find(&list, "sub/deep/u.txt"), &FileChange { status: '?', path: "sub/deep/u.txt".into(), added: 3, removed: 0 });
        assert_eq!(find(&list, "bin.dat").added, 0);

        // 在子目录里调用，路径仍相对仓库根
        let sub = changed_files(&d.join("sub"));
        assert_eq!(find(&sub, "sub/deep/u.txt").added, 3);
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn parse_rename_fields() {
        let list = parse_status(b"R  new\0old\0 M x\0?? y\0");
        let st: Vec<_> = list.iter().map(|c| (c.status, c.path.as_str())).collect();
        assert_eq!(st, [('R', "new"), ('M', "x"), ('?', "y")]);
        let mut m = HashMap::new();
        add_numstat(&mut m, b"1\t2\t\0old\0new\0-\t-\tbin\x003\t0\tf\0");
        assert_eq!(m["new"], (1, 2));
        assert_eq!(m["bin"], (0, 0));
        assert_eq!(m["f"], (3, 0));
    }
}
