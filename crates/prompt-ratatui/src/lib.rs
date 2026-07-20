use {
  prompt::{BoxedError, Prompter},
  ratatui::{
    Terminal, TerminalOptions, Viewport,
    backend::CrosstermBackend,
    crossterm::{
      event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
      terminal::{disable_raw_mode, enable_raw_mode}
    },
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Paragraph, Widget}
  },
  std::io::{self, Stdout}
};

/// Height of the inline viewport the live prompt is rendered in.
const VIEWPORT_HEIGHT: u16 = 6;

/// A [`Prompter`] rendering polished prompts with Ratatui : the live question sits in a small
/// inline viewport at the bottom of the terminal (the scrollback is preserved), and each answer
/// gets committed above it as a compact history line.
pub struct RatatuiPrompter {
  terminal: Terminal<CrosstermBackend<Stdout>>
}

impl RatatuiPrompter {
  pub fn new() -> Result<Self, BoxedError> {
    enable_raw_mode()?;

    let terminal = Terminal::with_options(CrosstermBackend::new(io::stdout()),
                                          TerminalOptions { viewport:
                                                              Viewport::Inline(VIEWPORT_HEIGHT) })?;

    Ok(Self { terminal })
  }

  /// Commits an answered question to the terminal scrollback, above the live prompt.
  fn record(&mut self, subject: &str, answer: Span<'static>) -> Result<(), BoxedError> {
    let history_line = Line::from(vec![Span::styled("  ✓ ", Style::new().fg(Color::Green)),
                                       Span::styled(format!("{subject}  "),
                                                    Style::new().fg(Color::DarkGray)),
                                       answer]);

    self.terminal
        .insert_before(1, |buffer| history_line.render(buffer.area, buffer))?;
    Ok(())
  }
}

impl Prompter for RatatuiPrompter {
  fn ask(&mut self, question: &str) -> Result<String, BoxedError> {
    let (subject, hint) = split_question(question);

    let mut input = String::new();
    loop {
      self.terminal.draw(|frame| {
        let card = question_card("enter to confirm · esc to abort");
        let content_area = card.inner(frame.area());
        frame.render_widget(card, frame.area());

        let content =
          Paragraph::new(vec![subject_line(&subject),
                              hint_line(&hint),
                              Line::default(),
                              Line::from(vec![Span::styled("❯ ",
                                                           Style::new().fg(Color::Green)
                                                                       .add_modifier(Modifier::BOLD)),
                                              Span::raw(input.clone()),
                                              Span::styled("▌", Style::new().fg(Color::Cyan))])]);
        frame.render_widget(content, content_area);
      })?;

      if let Event::Key(key) = event::read()?
         && key.kind == KeyEventKind::Press
      {
        match key.code {
          | KeyCode::Enter => {
            let answer = input.trim().to_string();

            // An empty answer either skips an optional value, or gets re-asked for a required
            // one : neither is worth a history line.
            if !answer.is_empty() {
              self.record(&subject, Span::styled(answer.clone(), Style::new().fg(Color::White)))?;
            }

            return Ok(answer);
          },

          | KeyCode::Backspace => {
            input.pop();
          },

          | KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) =>
            return Err("Setup wizard aborted".into()),
          | KeyCode::Esc => return Err("Setup wizard aborted".into()),

          | KeyCode::Char(character) => input.push(character),

          | _ => {}
        }
      }
    }
  }

  fn confirm(&mut self, question: &str) -> Result<bool, BoxedError> {
    let (subject, _) = split_question(question);

    let mut choice = true;
    loop {
      self.terminal.draw(|frame| {
        let card = question_card("←/→ or y/n · enter to confirm · esc to abort");
        let content_area = card.inner(frame.area());
        frame.render_widget(card, frame.area());

        let content = Paragraph::new(vec![subject_line(&subject),
                                          Line::default(),
                                          Line::from(vec![Span::raw("  "),
                                                          choice_pill("Yes", choice),
                                                          Span::raw("   "),
                                                          choice_pill("No", !choice)])]);
        frame.render_widget(content, content_area);
      })?;

      if let Event::Key(key) = event::read()?
         && key.kind == KeyEventKind::Press
      {
        let commit = |choice: bool, prompter: &mut Self| -> Result<bool, BoxedError> {
          let (answer, color) = if choice { ("Yes", Color::Green) } else { ("No", Color::Red) };
          prompter.record(&subject, Span::styled(answer, Style::new().fg(color)))?;
          Ok(choice)
        };

        match key.code {
          | KeyCode::Enter => return commit(choice, self),

          | KeyCode::Char('y') | KeyCode::Char('Y') => return commit(true, self),
          | KeyCode::Char('n') | KeyCode::Char('N') => return commit(false, self),

          | KeyCode::Left | KeyCode::Right | KeyCode::Tab => choice = !choice,

          | KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) =>
            return Err("Setup wizard aborted".into()),
          | KeyCode::Esc => return Err("Setup wizard aborted".into()),

          | _ => {}
        }
      }
    }
  }
}

impl Drop for RatatuiPrompter {
  fn drop(&mut self) {
    // Clear the inline viewport (the answer history above it stays in the scrollback), and hand
    // the terminal back.
    let _ = self.terminal.clear();
    let _ = self.terminal.show_cursor();
    let _ = disable_raw_mode();
  }
}

/// Splits a question like `repositories.kubeaid.url (press enter to skip)` into its subject and
/// its parenthesized hint.
fn split_question(question: &str) -> (String, String) {
  match question.split_once(" (") {
    | Some((subject, hint)) => (subject.to_string(), hint.trim_end_matches(')').to_string()),

    | None => (question.to_string(), String::new())
  }
}

/// The rounded-bordered card framing every question, with the key bindings as its footer.
fn question_card(key_bindings: &str) -> Block<'_> {
  Block::bordered().border_type(BorderType::Rounded)
                   .border_style(Style::new().fg(Color::DarkGray))
                   .title(Span::styled(" Setup wizard ",
                                       Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)))
                   .title_bottom(Line::from(Span::styled(format!(" {key_bindings} "),
                                                         Style::new().fg(Color::DarkGray)))
                                   .right_aligned())
}

fn subject_line(subject: &str) -> Line<'static> {
  Line::from(Span::styled(format!(" {subject}"),
                          Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)))
}

fn hint_line(hint: &str) -> Line<'static> {
  Line::from(Span::styled(format!(" {hint}"),
                          Style::new().fg(Color::DarkGray).add_modifier(Modifier::ITALIC)))
}

fn choice_pill(label: &str, selected: bool) -> Span<'static> {
  let style = if selected {
    Style::new().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD)
  } else {
    Style::new().fg(Color::DarkGray)
  };

  Span::styled(format!("  {label}  "), style)
}
