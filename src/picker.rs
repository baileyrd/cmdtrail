use anyhow::Result;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyModifiers},
    execute, queue,
    style::{Print, ResetColor, SetAttribute, Attribute},
    terminal::{self, ClearType},
};
use std::io::{stderr, Write};

/// RAII guard for the picker's terminal takeover: enables raw mode and
/// hides the cursor on construction, and unconditionally restores both
/// (plus clearing everything the picker drew) on drop — including on
/// early return via `?` from anywhere in `pick`. Without this, a
/// mid-render I/O error would strand the invoking shell's terminal in
/// raw mode with no visible cursor.
struct TerminalGuard;

impl TerminalGuard {
    fn new() -> Result<Self> {
        terminal::enable_raw_mode()?;
        if let Err(e) = execute!(stderr(), cursor::Hide) {
            let _ = terminal::disable_raw_mode();
            return Err(e.into());
        }
        Ok(TerminalGuard)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        // Best-effort: we're in a Drop impl, so errors here have nowhere
        // to go. The picker always leaves the cursor back on the row it
        // started at (see `render`'s trailing `MoveUp`), so clearing
        // everything from there down removes exactly what was drawn,
        // regardless of how many rows that ended up being.
        let mut out = stderr();
        let _ = queue!(
            out,
            cursor::MoveToColumn(0),
            terminal::Clear(ClearType::FromCursorDown)
        );
        let _ = execute!(out, cursor::Show, ResetColor);
        let _ = terminal::disable_raw_mode();
    }
}

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

    let _guard = TerminalGuard::new()?;

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
                    selected = selected.saturating_sub(1);
                }
                KeyCode::Char(c) => {
                    query.push(c);
                    selected = 0;
                }
                _ => {}
            }
        }
    };

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
