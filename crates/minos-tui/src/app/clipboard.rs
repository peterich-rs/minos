#[cfg(test)]
pub(super) static TEST_CLIPBOARD: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Serializes tests that read/write [`TEST_CLIPBOARD`] so parallel `cargo test`
/// runs cannot interleave clipboard mutations (flaky empty/non-empty asserts).
/// Async mutex so isolation can span the short `.await` points in those tests.
#[cfg(test)]
pub(super) static TEST_CLIPBOARD_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[cfg(test)]
pub(super) fn copy_to_clipboard(text: &str) -> anyhow::Result<()> {
    TEST_CLIPBOARD
        .lock()
        .expect("test clipboard lock")
        .push(text.to_owned());
    Ok(())
}

#[cfg(not(test))]
pub(super) fn copy_to_clipboard(text: &str) -> anyhow::Result<()> {
    if text.is_empty() {
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    const COMMANDS: &[(&str, &[&str])] = &[("pbcopy", &[])];
    #[cfg(target_os = "linux")]
    const COMMANDS: &[(&str, &[&str])] = &[
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["--clipboard", "--input"]),
    ];
    #[cfg(target_os = "windows")]
    const COMMANDS: &[(&str, &[&str])] =
        &[("powershell", &["-NoProfile", "-Command", "Set-Clipboard"])];
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    const COMMANDS: &[(&str, &[&str])] = &[];

    let mut last_error = None;
    for (program, args) in COMMANDS {
        match run_clipboard_command(program, args, text) {
            Ok(true) => return Ok(()),
            Ok(false) => {
                last_error = Some(anyhow::anyhow!("{program} exited with a non-zero status"));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => last_error = Some(error.into()),
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("no clipboard command available")))
}

#[cfg(not(test))]
fn run_clipboard_command(program: &str, args: &[&str], text: &str) -> std::io::Result<bool> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes())?;
    }
    Ok(child.wait()?.success())
}

#[cfg(not(test))]
pub(super) fn paste_from_clipboard() -> anyhow::Result<String> {
    #[cfg(target_os = "macos")]
    const COMMANDS: &[(&str, &[&str])] = &[("pbpaste", &[])];
    #[cfg(target_os = "linux")]
    const COMMANDS: &[(&str, &[&str])] = &[
        ("wl-paste", &[]),
        ("xclip", &["-selection", "clipboard", "-o"]),
        ("xsel", &["--clipboard", "--output"]),
    ];
    #[cfg(target_os = "windows")]
    const COMMANDS: &[(&str, &[&str])] =
        &[("powershell", &["-NoProfile", "-Command", "Get-Clipboard"])];
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    const COMMANDS: &[(&str, &[&str])] = &[];

    let mut last_error = None;
    for (program, args) in COMMANDS {
        match run_paste_command(program, args) {
            Ok(output) if !output.is_empty() => {
                return Ok(crate::input::normalize_pasted_text(&output));
            }
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => last_error = Some(error.into()),
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("no clipboard command available")))
}

#[cfg(not(test))]
fn run_paste_command(program: &str, args: &[&str]) -> std::io::Result<String> {
    let output = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()?;
    String::from_utf8(output.stdout)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "non-utf8 clipboard"))
}

#[cfg(test)]
pub(super) fn paste_from_clipboard() -> anyhow::Result<String> {
    TEST_CLIPBOARD
        .lock()
        .expect("test clipboard lock")
        .last()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("clipboard empty"))
}
