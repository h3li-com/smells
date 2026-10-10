use std::{env, process::ExitCode};

fn main() -> ExitCode {
    let args = env::args().skip(1).collect();
    match smells::run(args) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("smells: {error}");
            ExitCode::from(2)
        }
    }
}
