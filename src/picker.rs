use anyhow::Result;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyModifiers},
    execute, queue,
    style::{Print, ResetColor, SetAttribute, Attribute},
    terminal::{self, ClearType},
};
use std::io::{stderr, Write};

/// Renders `items` (best-first) to stderr with a type-to-filter prompt.
/// Returns the chosen string on Enter, or None on Esc/Ctrl-C.
///
/// Rendered on stderr / read from the tty directly so stdout stays clean —
/// callers (the shell hook) capture only the final chosen command from
/// stdout.
pub fn pick(items: &[String]) -> Result<Option<String>> {
    if items.is_empty() {
        return Ok(None);
    }

    let mut out = stderr();
    terminal::enable_raw_mode()?;
    execute!(out, cursor::Hide)?;

    let mut query = String::new();
    let mut selected: usize = 0;
    let result = loop {
        let filtered: Vec<&String> = items
            .iter()
            .filter(|c| c.to_lowercase().contains(&query.to_lowercase()))
            .collect();
        if selected >= filtered.len() && !filtered.is_empty() {
            selected = filtered.len() - 1;
        }

        render(&query, &filtered, selected)?;

        if let Event::Key(key) = event::read()? {
            match key.code {
                KeyCode::Enter => {
                    break filtered.get(selected).map(|s| s.to_string());
                }
                KeyCode::Esc => break None,
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break None,
                KeyCode::Backspace => {
                    query.pop();
                    selected = 0;
                }
                KeyCode::Down => {
                    if selected + 1 < filtered.len() {
                        selected += 1;
                    }
                }
                KeyCode::Up => {
                    if selected > 0 {
                        selected -= 1;
                    }
                }
                KeyCode::Char(c) => {
                    query.push(c);
                    selected = 0;
                }
                _ => {}
            }
        }
    };

    // Clean up: clear the lines we drew.
    let lines_drawn = (items.len().min(10) + 1) as u16;
    queue!(out, cursor::MoveToColumn(0))?;
    for _ in 0..lines_drawn {
        queue!(out, terminal::Clear(ClearType::CurrentLine), cursor::MoveUp(1))?;
    }
    queue!(out, terminal::Clear(ClearType::CurrentLine))?;
    execute!(out, cursor::Show, ResetColor)?;
    terminal::disable_raw_mode()?;

    Ok(result)
}

fn render(query: &str, filtered: &[&String], selected: usize) -> Result<()> {
    let mut out = stderr();
    queue!(out, cursor::MoveToColumn(0), terminal::Clear(ClearType::FromCursorDown))?;
    queue!(out, Print(format!("cmdtrail> {}\r\n", query)))?;

    for (i, item) in filtered.iter().take(10).enumerate() {
        if i == selected {
            queue!(out, SetAttribute(Attribute::Reverse), Print(format!("  {}\r\n", item)), ResetColor)?;
        } else {
            queue!(out, Print(format!("  {}\r\n", item)))?;
        }
    }
    // Move cursor back up to just under the prompt line for the next redraw.
    let drawn = filtered.len().min(10) as u16;
    queue!(out, cursor::MoveUp(drawn + 1))?;
    out.flush()?;
    Ok(())
}
