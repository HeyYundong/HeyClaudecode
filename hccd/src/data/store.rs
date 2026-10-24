//! 项目列表与 hcc 共用 ~/.config/heyclaude/projects.json；不关联项目的对话放在数据目录下。

use std::cmp::Ordering;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, anyhow, bail};
use serde_json::{Map, Value, json};

use crate::model::Project;

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

/// 环境变量非空时用它，否则用 ~/<fallback>
fn xdg_dir(var: &str, fallback: &str) -> PathBuf {
    match std::env::var_os(var) {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => home().join(fallback),
    }
}

/// $XDG_CONFIG_HOME/heyclaude，和 hcc 的 configDir() 一致
fn config_dir() -> PathBuf {
    xdg_dir("XDG_CONFIG_HOME", ".config").join("heyclaude")
}

fn projects_file() -> PathBuf {
    config_dir().join("projects.json")
}

pub fn load_projects() -> anyhow::Result<Vec<Project>> {
    load_from(&projects_file())
}

/// 校验目录、去重、按名字排序后写回文件，返回新列表
pub fn add_project(projects: &[Project], raw_path: &str) -> anyhow::Result<Vec<Project>> {
    let list = with_added(projects, raw_path)?;
    save_to(&projects_file(), &list)?;
    Ok(list)
}

pub fn remove_project(projects: &[Project], path: &Path) -> anyhow::Result<Vec<Project>> {
    let list: Vec<Project> = projects.iter().filter(|p| p.path != path).cloned().collect();
    save_to(&projects_file(), &list)?;
    Ok(list)
}

/// 不关联项目的对话的根目录：$XDG_DATA_HOME/heyclaude/chats（默认 ~/.local/share/heyclaude/chats）
pub fn chats_root() -> PathBuf {
    xdg_dir("XDG_DATA_HOME", ".local/share").join("heyclaude").join("chats")
}

/// 为一个新对话建一个空目录并返回
pub fn new_chat_dir() -> anyhow::Result<PathBuf> {
    let dir = chats_root().join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&dir).with_context(|| format!("无法创建目录: {}", dir.display()))?;
    Ok(dir)
}

fn load_from(file: &Path) -> anyhow::Result<Vec<Project>> {
    let text = match fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => bail!("无法读取 {}: {e}", file.display()),
    };
    let data: Value = serde_json::from_str(&text).map_err(|e| anyhow!("无法读取 {}: {e}", file.display()))?;
    let mut list: Vec<Project> = data
        .get("projects")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|p| {
                    Some(Project {
                        name: p.get("name")?.as_str()?.to_string(),
                        path: PathBuf::from(p.get("path")?.as_str()?),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    sort_projects(&mut list);
    Ok(list)
}

/// 写回时以文件里已有的条目为底，保留 addedAt 和其他未知字段；原子写
fn save_to(file: &Path, projects: &[Project]) -> anyhow::Result<()> {
    let mut root = fs::read_to_string(file)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| match v {
            Value::Object(m) => Some(m),
            _ => None,
        })
        .unwrap_or_default();
    let old: Vec<Value> = root.get("projects").and_then(Value::as_array).cloned().unwrap_or_default();

    let items: Vec<Value> = projects
        .iter()
        .map(|p| {
            let path = p.path.to_string_lossy();
            let mut obj = old
                .iter()
                .find(|o| o.get("path").and_then(Value::as_str) == Some(&*path))
                .and_then(|o| o.as_object().cloned())
                .unwrap_or_else(|| {
                    let mut m = Map::new();
                    m.insert("addedAt".into(), json!(iso_now()));
                    m
                });
            obj.insert("name".into(), json!(p.name));
            obj.insert("path".into(), json!(path));
            Value::Object(obj)
        })
        .collect();
    root.insert("projects".into(), Value::Array(items));

    let dir = file.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir).with_context(|| format!("无法创建目录: {}", dir.display()))?;
    let tmp = file.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(&Value::Object(root))? + "\n";
    fs::write(&tmp, text).with_context(|| format!("无法写入 {}", tmp.display()))?;
    fs::rename(&tmp, file).with_context(|| format!("无法写入 {}", file.display()))?;
    Ok(())
}

fn with_added(projects: &[Project], raw_path: &str) -> anyhow::Result<Vec<Project>> {
    let resolved = expand_path(raw_path);
    let meta = fs::metadata(&resolved).map_err(|_| anyhow!("目录不存在: {}", resolved.display()))?;
    if !meta.is_dir() {
        bail!("不是目录: {}", resolved.display());
    }
    if projects.iter().any(|p| p.path == resolved) {
        bail!("已经添加过: {}", resolved.display());
    }
    let name = resolved
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| resolved.to_string_lossy().into_owned());
    let mut list = projects.to_vec();
    list.push(Project { name, path: resolved });
    sort_projects(&mut list);
    Ok(list)
}

/// 展开 ~ 并转成绝对路径（只做字面上的规范化，不解析符号链接，同 node 的 path.resolve）；空串表示当前目录
fn expand_path(raw: &str) -> PathBuf {
    let p = raw.trim();
    let path = if p == "~" {
        home()
    } else if let Some(rest) = p.strip_prefix("~/") {
        home().join(rest)
    } else {
        PathBuf::from(p)
    };
    let abs = if path.is_absolute() {
        path
    } else {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")).join(path)
    };
    let mut out = PathBuf::new();
    for c in abs.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// 按名字排序，不区分大小写，数字按数值比较（project2 在 project10 前面）
fn sort_projects(list: &mut [Project]) {
    list.sort_by(|a, b| natural_cmp(&a.name, &b.name));
}

fn natural_cmp(a: &str, b: &str) -> Ordering {
    let x: Vec<char> = a.chars().flat_map(char::to_lowercase).collect();
    let y: Vec<char> = b.chars().flat_map(char::to_lowercase).collect();
    let (mut i, mut j) = (0, 0);
    while i < x.len() && j < y.len() {
        if x[i].is_ascii_digit() && y[j].is_ascii_digit() {
            let (si, sj) = (i, j);
            while i < x.len() && x[i].is_ascii_digit() {
                i += 1;
            }
            while j < y.len() && y[j].is_ascii_digit() {
                j += 1;
            }
            // 去掉前导零后先比位数再逐位比
            let na: Vec<char> = x[si..i].iter().copied().skip_while(|&c| c == '0').collect();
            let nb: Vec<char> = y[sj..j].iter().copied().skip_while(|&c| c == '0').collect();
            let ord = na.len().cmp(&nb.len()).then_with(|| na.cmp(&nb));
            if ord != Ordering::Equal {
                return ord;
            }
        } else {
            let ord = x[i].cmp(&y[j]);
            if ord != Ordering::Equal {
                return ord;
            }
            i += 1;
            j += 1;
        }
    }
    (x.len() - i).cmp(&(y.len() - j)).then_with(|| a.cmp(b))
}

/// 当前时间的 ISO 8601 UTC 字符串，格式同 JS 的 toISOString()
fn iso_now() -> String {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    iso_from_millis(d.as_millis() as i64)
}

fn iso_from_millis(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    // Howard Hinnant 的 civil_from_days
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
        ms.rem_euclid(1000)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("hccd-store-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn proj(name: &str) -> Project {
        Project { name: name.into(), path: PathBuf::from(format!("/x/{name}")) }
    }

    #[test]
    fn iso_format() {
        assert_eq!(iso_from_millis(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_from_millis(1_709_210_096_789), "2024-02-29T12:34:56.789Z");
        assert_eq!(iso_from_millis(951_782_400_000), "2000-02-29T00:00:00.000Z");
    }

    #[test]
    fn natural_sort() {
        let mut l = vec![proj("project10"), proj("Beta"), proj("project2"), proj("alpha")];
        sort_projects(&mut l);
        let names: Vec<_> = l.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["alpha", "Beta", "project2", "project10"]);
    }

    #[test]
    fn expand() {
        assert_eq!(expand_path("~"), home());
        assert_eq!(expand_path("~/a/../b"), home().join("b"));
        assert_eq!(expand_path("/a/./b/../c/"), PathBuf::from("/a/c"));
    }

    #[test]
    fn add_validates() {
        let dir = temp_dir();
        let file = dir.join("f.txt");
        fs::write(&file, "x").unwrap();
        let sub = dir.join("Sub");
        fs::create_dir(&sub).unwrap();

        let err = with_added(&[], dir.join("missing").to_str().unwrap()).unwrap_err();
        assert!(err.to_string().starts_with("目录不存在"));
        let err = with_added(&[], file.to_str().unwrap()).unwrap_err();
        assert!(err.to_string().starts_with("不是目录"));
        let list = with_added(&[], &format!("{}/", sub.display())).unwrap();
        assert_eq!(list, vec![Project { name: "Sub".into(), path: sub.clone() }]);
        let err = with_added(&list, sub.to_str().unwrap()).unwrap_err();
        assert!(err.to_string().starts_with("已经添加过"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn save_keeps_unknown_fields() {
        let dir = temp_dir();
        let file = dir.join("heyclaude/projects.json");
        assert!(load_from(&file).unwrap().is_empty());
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(
            &file,
            r#"{"projects":[{"name":"b","path":"/x/b","addedAt":"2020-01-01T00:00:00.000Z","color":"red"},
                {"name":"a","path":"/x/a","addedAt":"2021-01-01T00:00:00.000Z"}],"other":1}"#,
        )
        .unwrap();
        let loaded = load_from(&file).unwrap();
        assert_eq!(loaded, vec![proj("a"), proj("b")]);

        let list = vec![proj("b"), proj("c")];
        save_to(&file, &list).unwrap();
        let v: Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(v["other"], 1);
        let ps = v["projects"].as_array().unwrap();
        assert_eq!(ps.len(), 2);
        assert_eq!(ps[0]["addedAt"], "2020-01-01T00:00:00.000Z");
        assert_eq!(ps[0]["color"], "red");
        assert_eq!(ps[1]["path"], "/x/c");
        assert!(ps[1]["addedAt"].as_str().unwrap().ends_with('Z'));
        assert!(!file.with_extension("json.tmp").exists());
        assert_eq!(load_from(&file).unwrap(), list);
        fs::remove_dir_all(dir).unwrap();
    }
}
