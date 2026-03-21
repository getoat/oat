use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let config = oat::config::AppConfig::load_from_default_path()?;
    let mut terminal = oat::setup_terminal()?;
    let result = oat::run(&mut terminal, config);
    oat::restore_terminal(&mut terminal)?;
    result
}
