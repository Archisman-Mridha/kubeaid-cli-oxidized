pub use prompt_derive::Prompt;

use std::{collections::VecDeque,
          io::{self, Write},
          path::PathBuf};

pub type BoxedError = Box<dyn std::error::Error>;

/// Asks the user questions over some interactive medium (usually stdin / stdout).
pub trait Prompter {
  /// Asks a free-form question, returning the raw answer.
  fn ask(&mut self, question: &str) -> Result<String, BoxedError>;

  /// Asks a yes / no question.
  fn confirm(&mut self, question: &str) -> Result<bool, BoxedError>;
}

/// A value which can be constructed by interactively prompting the user for its parts.
///
/// Derive it (`#[derive(Prompt)]`) on config structs : each field turns into a CLI prompt
/// automatically, so adding / removing fields keeps the wizard in sync.
pub trait Prompt: Sized {
  /// Prompts for the value from scratch.
  fn prompt(prompter: &mut dyn Prompter, path: &str) -> Result<Self, BoxedError>;

  /// Prompts for the value, letting the user skip it.
  fn prompt_optional(prompter: &mut dyn Prompter, path: &str) -> Result<Option<Self>, BoxedError>;

  /// Keeps the existing value when present, prompting for it otherwise. Derived structs override
  /// this to recurse, prompting only for their missing parts.
  fn prompt_or_keep(existing: Option<Self>,
                    prompter: &mut dyn Prompter,
                    path: &str)
                    -> Result<Self, BoxedError> {
    match existing {
      | Some(value) => Ok(value),

      | None => Self::prompt(prompter, path)
    }
  }
}

impl Prompt for String {
  fn prompt(prompter: &mut dyn Prompter, path: &str) -> Result<Self, BoxedError> {
    loop {
      let answer = prompter.ask(path)?;

      let answer = answer.trim();
      if !answer.is_empty() {
        return Ok(answer.to_string());
      }
    }
  }

  fn prompt_optional(prompter: &mut dyn Prompter, path: &str) -> Result<Option<Self>, BoxedError> {
    let answer = prompter.ask(&format!("{path} (press enter to skip)"))?;

    let answer = answer.trim();
    Ok((!answer.is_empty()).then(|| answer.to_string()))
  }
}

impl Prompt for PathBuf {
  fn prompt(prompter: &mut dyn Prompter, path: &str) -> Result<Self, BoxedError> {
    Ok(String::prompt(prompter, path)?.into())
  }

  fn prompt_optional(prompter: &mut dyn Prompter, path: &str) -> Result<Option<Self>, BoxedError> {
    Ok(String::prompt_optional(prompter, path)?.map(Into::into))
  }
}

impl Prompt for Vec<String> {
  fn prompt(prompter: &mut dyn Prompter, path: &str) -> Result<Self, BoxedError> {
    loop {
      let answer = prompter.ask(&format!("{path} (comma separated)"))?;

      let values = split_comma_separated(&answer);
      if !values.is_empty() {
        return Ok(values);
      }
    }
  }

  fn prompt_optional(prompter: &mut dyn Prompter, path: &str) -> Result<Option<Self>, BoxedError> {
    let answer = prompter.ask(&format!("{path} (comma separated, press enter to skip)"))?;

    let values = split_comma_separated(&answer);
    Ok((!values.is_empty()).then_some(values))
  }
}

fn split_comma_separated(answer: &str) -> Vec<String> {
  answer.split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

/// Builds the dot separated path of a field within the config (for example
/// `repositories.sshAccess.knownHosts`).
pub fn child_path(parent: &str, field: &str) -> String {
  if parent.is_empty() {
    field.to_string()
  } else {
    format!("{parent}.{field}")
  }
}

/// A [`Prompter`] over stdin / stdout.
pub struct StdioPrompter;

impl Prompter for StdioPrompter {
  fn ask(&mut self, question: &str) -> Result<String, BoxedError> {
    print!("{question} : ");
    io::stdout().flush()?;

    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(answer)
  }

  fn confirm(&mut self, question: &str) -> Result<bool, BoxedError> {
    loop {
      match self.ask(&format!("{question} [y/n]"))?.trim().to_ascii_lowercase().as_str() {
        | "y" | "yes" => return Ok(true),
        | "n" | "no" => return Ok(false),

        | _ => {}
      }
    }
  }
}

/// A [`Prompter`] which replays pre-scripted answers. Meant for tests.
pub struct ScriptedPrompter {
  answers: VecDeque<String>
}

impl ScriptedPrompter {
  pub fn new<Answers: IntoIterator>(answers: Answers) -> Self
    where Answers::Item: Into<String> {
    Self { answers: answers.into_iter().map(Into::into).collect() }
  }

  fn next_answer(&mut self, question: &str) -> Result<String, BoxedError> {
    self.answers
        .pop_front()
        .ok_or_else(|| format!("No scripted answer left for the question : {question}").into())
  }
}

impl Prompter for ScriptedPrompter {
  fn ask(&mut self, question: &str) -> Result<String, BoxedError> {
    self.next_answer(question)
  }

  fn confirm(&mut self, question: &str) -> Result<bool, BoxedError> {
    match self.next_answer(question)?.trim().to_ascii_lowercase().as_str() {
      | "y" | "yes" => Ok(true),
      | "n" | "no" => Ok(false),

      | answer =>
        Err(format!("Scripted answer `{answer}` to the question `{question}` isn't a yes / no").into())
    }
  }
}
