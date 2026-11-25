use std::process::ExitCode;

use heyclaude::backend::Backend;

fn main() -> ExitCode {
	heyclaude::main(Backend::Agy)
}
