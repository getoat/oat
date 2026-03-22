use std::{
    env,
    error::Error,
    io::{self, Write},
    process,
};

use oat::{
    StartupOptions,
    app::{AccessMode, ApprovalMode},
};

#[derive(Debug, PartialEq, Eq)]
struct CliOptions {
    headless: bool,
    dangerous: bool,
    prompt: Option<String>,
}

impl CliOptions {
    fn startup_options(&self) -> StartupOptions {
        if self.dangerous {
            StartupOptions {
                access_mode: AccessMode::ReadWrite,
                approval_mode: ApprovalMode::Disabled,
            }
        } else {
            StartupOptions::default()
        }
    }
}

fn main() {
    if let Err(error) = try_main() {
        eprintln!("{error}");
        process::exit(1);
    }
}

fn try_main() -> Result<(), Box<dyn Error>> {
    let cli = parse_args(env::args().skip(1))
        .map_err(|message| io::Error::new(io::ErrorKind::InvalidInput, message))?;
    let config = oat::config::AppConfig::load_from_default_path()?;
    let startup = cli.startup_options();

    if cli.headless {
        let output = oat::run_headless(config, startup, cli.prompt.expect("headless prompt"))?;
        print!("{output}");
        io::stdout().flush()?;
        return Ok(());
    }

    let mut terminal = oat::setup_terminal()?;
    let result = oat::run_with_options(&mut terminal, config, startup);
    oat::restore_terminal(&mut terminal)?;
    result
}

fn parse_args<I>(args: I) -> Result<CliOptions, String>
where
    I: IntoIterator<Item = String>,
{
    let mut headless = false;
    let mut dangerous = false;
    let mut positionals = Vec::new();
    let mut parsing_flags = true;

    for arg in args {
        if parsing_flags && arg == "--" {
            parsing_flags = false;
            continue;
        }

        if parsing_flags && arg.starts_with("--") {
            match arg.as_str() {
                "--headless" => headless = true,
                "--dangerous" => dangerous = true,
                _ => return Err(format!("Unknown flag `{arg}`.\n\n{}", usage())),
            }
        } else {
            positionals.push(arg);
        }
    }

    if headless {
        if positionals.len() != 1 {
            return Err(format!(
                "Headless mode requires exactly one quoted prompt argument.\n\n{}",
                usage()
            ));
        }

        Ok(CliOptions {
            headless,
            dangerous,
            prompt: positionals.into_iter().next(),
        })
    } else if positionals.is_empty() {
        Ok(CliOptions {
            headless,
            dangerous,
            prompt: None,
        })
    } else {
        Err(format!(
            "Positional arguments are only supported with `--headless`.\n\n{}",
            usage()
        ))
    }
}

fn usage() -> &'static str {
    "Usage:\n  oat\n  oat --dangerous\n  oat --headless \"prompt\"\n  oat --headless --dangerous \"prompt\""
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(values: &[&str]) -> Result<CliOptions, String> {
        parse_args(values.iter().map(|value| (*value).to_string()))
    }

    #[test]
    fn parses_default_tui_startup() {
        assert_eq!(
            parse(&[]).expect("parse succeeds"),
            CliOptions {
                headless: false,
                dangerous: false,
                prompt: None,
            }
        );
    }

    #[test]
    fn parses_dangerous_tui_startup() {
        assert_eq!(
            parse(&["--dangerous"]).expect("parse succeeds"),
            CliOptions {
                headless: false,
                dangerous: true,
                prompt: None,
            }
        );
    }

    #[test]
    fn parses_headless_prompt() {
        assert_eq!(
            parse(&["--headless", "fix the tests"]).expect("parse succeeds"),
            CliOptions {
                headless: true,
                dangerous: false,
                prompt: Some("fix the tests".into()),
            }
        );
    }

    #[test]
    fn rejects_headless_without_prompt() {
        let error = parse(&["--headless"]).expect_err("parse should fail");
        assert!(error.contains("exactly one quoted prompt"));
    }

    #[test]
    fn rejects_multiple_headless_positionals() {
        let error = parse(&["--headless", "one", "two"]).expect_err("parse should fail");
        assert!(error.contains("exactly one quoted prompt"));
    }

    #[test]
    fn rejects_positional_args_without_headless() {
        let error = parse(&["hello"]).expect_err("parse should fail");
        assert!(error.contains("only supported with `--headless`"));
    }

    #[test]
    fn rejects_unknown_flags() {
        let error = parse(&["--wat"]).expect_err("parse should fail");
        assert!(error.contains("Unknown flag `--wat`"));
    }
}
