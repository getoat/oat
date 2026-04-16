use std::{
    error::Error,
    io::{self, Write},
    path::PathBuf,
    process,
};

use clap::Parser;
use oat::{HeadlessMode, HeadlessOverrides, StartupOptions};

#[derive(Debug, Parser, PartialEq, Eq)]
#[command(name = "oat")]
struct CliOptions {
    #[arg(long, conflicts_with = "headless_plan")]
    headless: bool,
    #[arg(long, conflicts_with = "headless")]
    headless_plan: bool,
    #[arg(long)]
    auto_accept_plan: bool,
    #[arg(long)]
    dangerous: bool,
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    reasoning: Option<String>,
    #[arg(long = "planning-agent")]
    planning_agents: Vec<String>,
    prompt: Option<String>,
}

impl CliOptions {
    fn startup_options(&self) -> StartupOptions {
        if self.dangerous {
            StartupOptions::dangerous()
        } else {
            StartupOptions::default()
        }
    }

    fn headless_mode(&self) -> Option<HeadlessMode> {
        match (self.headless, self.headless_plan, self.auto_accept_plan) {
            (true, false, false) => Some(HeadlessMode::Prompt),
            (false, true, false) => Some(HeadlessMode::Plan),
            (false, true, true) => Some(HeadlessMode::PlanAndImplement),
            _ => None,
        }
    }

    fn validate(&self) -> Result<(), String> {
        let headless = self.headless || self.headless_plan;
        if headless && self.prompt.is_none() {
            return Err(format!(
                "Headless modes require exactly one quoted prompt argument.\n\n{}",
                usage()
            ));
        }
        if !headless && self.prompt.is_some() {
            return Err(format!(
                "Positional arguments are only supported with `--headless` or `--headless-plan`.\n\n{}",
                usage()
            ));
        }
        if self.auto_accept_plan && !self.headless_plan {
            return Err(format!(
                "`--auto-accept-plan` requires `--headless-plan`.\n\n{}",
                usage()
            ));
        }
        if !headless
            && (self.model.is_some()
                || self.reasoning.is_some()
                || !self.planning_agents.is_empty()
                || self.auto_accept_plan)
        {
            return Err(format!(
                "`--model`, `--reasoning`, `--planning-agent`, and `--auto-accept-plan` are only supported in headless modes.\n\n{}",
                usage()
            ));
        }
        Ok(())
    }
}

fn main() {
    if let Err(error) = try_main() {
        eprintln!("{error}");
        process::exit(1);
    }
}

fn try_main() -> Result<(), Box<dyn Error>> {
    let cli = CliOptions::try_parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error.to_string()))?;
    cli.validate()
        .map_err(|message| io::Error::new(io::ErrorKind::InvalidInput, message))?;
    let startup = cli.startup_options();

    if let Some(mode) = cli.headless_mode() {
        let output = oat::run_headless_with_options(
            startup,
            cli.config.as_deref(),
            HeadlessOverrides {
                model_name: cli.model,
                reasoning: cli.reasoning,
                planning_agents: cli.planning_agents,
            },
            mode,
            cli.prompt.expect("validated headless prompt"),
        )?;
        print!("{output}");
        io::stdout().flush()?;
        return Ok(());
    }

    let mut terminal = oat::setup_terminal()?;
    let result = oat::run_tui_with_config(&mut terminal, startup, cli.config.as_deref());
    oat::restore_terminal(&mut terminal)?;
    result
}

fn usage() -> &'static str {
    "Usage:\n  oat\n  oat --config config.toml\n  oat --dangerous\n  oat --headless \"prompt\"\n  oat --headless --model gpt-5.4 \"prompt\"\n  oat --headless-plan --planning-agent gpt-5.4::high \"prompt\"\n  oat --headless-plan --auto-accept-plan --dangerous \"prompt\""
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(values: &[&str]) -> Result<CliOptions, String> {
        let cli = CliOptions::try_parse_from(std::iter::once("oat").chain(values.iter().copied()))
            .map_err(|error| error.to_string())?;
        cli.validate()?;
        Ok(cli)
    }

    #[test]
    fn parses_default_tui_startup() {
        assert_eq!(
            parse(&[]).expect("parse succeeds"),
            CliOptions {
                headless: false,
                headless_plan: false,
                auto_accept_plan: false,
                dangerous: false,
                config: None,
                model: None,
                reasoning: None,
                planning_agents: Vec::new(),
                prompt: None,
            }
        );
    }

    #[test]
    fn parses_configured_tui_startup() {
        let parsed = parse(&["--config", "custom.toml"]).expect("parse succeeds");
        assert_eq!(parsed.config, Some(PathBuf::from("custom.toml")));
    }

    #[test]
    fn parses_headless_prompt() {
        assert_eq!(
            parse(&["--headless", "fix the tests"]).expect("parse succeeds"),
            CliOptions {
                headless: true,
                headless_plan: false,
                auto_accept_plan: false,
                dangerous: false,
                config: None,
                model: None,
                reasoning: None,
                planning_agents: Vec::new(),
                prompt: Some("fix the tests".into()),
            }
        );
    }

    #[test]
    fn parses_headless_plan_with_overrides() {
        let parsed = parse(&[
            "--headless-plan",
            "--auto-accept-plan",
            "--dangerous",
            "--model",
            "gpt-5.4",
            "--reasoning",
            "high",
            "--planning-agent",
            "gpt-5.4-mini::medium",
            "--planning-agent",
            "kimi-k2.5::on",
            "plan this",
        ])
        .expect("parse succeeds");

        assert_eq!(parsed.headless_mode(), Some(HeadlessMode::PlanAndImplement));
        assert_eq!(parsed.model.as_deref(), Some("gpt-5.4"));
        assert_eq!(parsed.reasoning.as_deref(), Some("high"));
        assert_eq!(
            parsed.planning_agents,
            vec!["gpt-5.4-mini::medium", "kimi-k2.5::on"]
        );
    }

    #[test]
    fn rejects_headless_without_prompt() {
        let error = parse(&["--headless"]).expect_err("parse should fail");
        assert!(error.contains("exactly one quoted prompt"));
    }

    #[test]
    fn rejects_positional_args_without_headless() {
        let error = parse(&["hello"]).expect_err("parse should fail");
        assert!(error.contains("only supported with `--headless` or `--headless-plan`"));
    }

    #[test]
    fn rejects_auto_accept_without_headless_plan() {
        let error = parse(&["--auto-accept-plan"]).expect_err("parse should fail");
        assert!(error.contains("requires `--headless-plan`"));
    }

    #[test]
    fn rejects_headless_only_flags_in_tui_mode() {
        let error = parse(&["--model", "gpt-5.4"]).expect_err("parse should fail");
        assert!(error.contains("only supported in headless modes"));
    }
}
