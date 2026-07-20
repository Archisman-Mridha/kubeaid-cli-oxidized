//! Runs the config setup wizard with the Ratatui prompter :
//!
//! ```sh
//! cargo run -p config --example wizard
//! ```

use {config::raw, prompt::Prompt, prompt_ratatui::RatatuiPrompter};

fn main() -> Result<(), Box<dyn std::error::Error>> {
  let mut prompter = RatatuiPrompter::new()?;
  let completed = raw::Config::prompt_or_keep(None, &mut prompter, "")?;
  drop(prompter); // Hand the terminal back before printing.

  println!("Completed config :\n{completed:#?}");
  Ok(())
}
