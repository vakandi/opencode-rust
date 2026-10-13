use anyhow::Context;
use base64::{engine::general_purpose::STANDARD, Engine};
use std::io::Write;
use std::process::{Command, Stdio};

pub struct ClipboardContent {
    pub data: String,
    pub mime: String,
}

pub struct Clipboard;

impl Clipboard {
    pub fn read() -> anyhow::Result<ClipboardContent> {
        if let Ok(image_data) = read_image_from_clipboard() {
            return Ok(ClipboardContent {
                data: image_data,
                mime: "image/png".to_string(),
            });
        }

        let text = Self::read_text()?;
        Ok(ClipboardContent {
            data: text,
            mime: "text/plain".to_string(),
        })
    }

    pub fn read_text() -> anyhow::Result<String> {
        if cfg!(target_os = "macos") {
            return read_with_command("pbpaste", &[]);
        }

        if cfg!(target_os = "windows") {
            return read_with_command(
                "powershell",
                &["-NoProfile", "-Command", "Get-Clipboard -Raw"],
            );
        }

        if std::env::var("WAYLAND_DISPLAY").is_ok() {
            if let Ok(text) = read_with_command("wl-paste", &["-n"]) {
                return Ok(text);
            }
        }

        if let Ok(text) = read_with_command("xclip", &["-selection", "clipboard", "-o"]) {
            return Ok(text);
        }

        read_with_command("xsel", &["--clipboard", "--output"])
    }

    pub fn write_text(text: &str) -> anyhow::Result<()> {
        // Always attempt OSC 52 first — works over SSH when the terminal
        // emulator supports it, even without native clipboard tools.
        write_osc52(text);

        if cfg!(target_os = "macos") {
            return write_with_command("pbcopy", &[], text);
        }

        if cfg!(target_os = "windows") {
            return write_with_command(
                "powershell",
                &[
                    "-NoProfile",
                    "-Command",
                    "[Console]::InputEncoding = [System.Text.Encoding]::UTF8; Set-Clipboard -Value ([Console]::In.ReadToEnd())",
                ],
                text,
            );
        }

        // Linux: try native clipboard tools, fall back to OSC 52 only.
        if std::env::var("WAYLAND_DISPLAY").is_ok() {
            if write_with_command("wl-copy", &[], text).is_ok() {
                return Ok(());
            }
        }

        if write_with_command("xclip", &["-selection", "clipboard"], text).is_ok() {
            return Ok(());
        }

        if write_with_command("xsel", &["--clipboard", "--input"], text).is_ok() {
            return Ok(());
        }

        // No native tool available — OSC 52 was already sent above,
        // so the clipboard write likely succeeded via the terminal emulator.
        Ok(())
    }
}

fn write_osc52(text: &str) {
    let encoded = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    let osc52 = format!("\x1b]52;c;{encoded}\x07");

    let sequence = if std::env::var("TMUX").is_ok() || std::env::var("STY").is_ok() {
        format!("\x1bPtmux;\x1b{osc52}\x1b\\")
    } else {
        osc52
    };

    let _ = std::io::stdout()
        .write_all(sequence.as_bytes())
        .and_then(|_| std::io::stdout().flush());
}

fn read_with_command(program: &str, args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("failed to execute clipboard read command: {program}"))?;

    if !output.status.success() {
        anyhow::bail!(
            "clipboard read command `{}` failed with status {}",
            program,
            output.status
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn write_with_command(program: &str, args: &[&str], text: &str) -> anyhow::Result<()> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("failed to execute clipboard write command: {program}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes()).with_context(|| {
            format!("failed to write text to clipboard command stdin: {program}")
        })?;
    }

    let status = child
        .wait()
        .with_context(|| format!("failed waiting for clipboard command: {program}"))?;
    if !status.success() {
        anyhow::bail!(
            "clipboard write command `{}` failed with status {}",
            program,
            status
        );
    }
    Ok(())
}

fn read_raw_with_command(program: &str, args: &[&str]) -> anyhow::Result<Vec<u8>> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("failed to execute clipboard image read command: {program}"))?;

    if !output.status.success() {
        anyhow::bail!(
            "clipboard image read command `{}` failed with status {}",
            program,
            output.status
        );
    }

    if output.stdout.is_empty() {
        anyhow::bail!(
            "clipboard image read command `{}` returned empty output",
            program
        );
    }

    Ok(output.stdout)
}

fn read_image_from_clipboard() -> anyhow::Result<String> {
    if cfg!(target_os = "macos") {
        return read_image_macos();
    }

    if cfg!(target_os = "windows") {
        return read_image_windows();
    }

    // Linux: try Wayland first, then X11
    if std::env::var("WAYLAND_DISPLAY").is_ok() {
        if let Ok(data) = read_raw_with_command("wl-paste", &["-t", "image/png"]) {
            return Ok(STANDARD.encode(&data));
        }
    }

    let data = read_raw_with_command(
        "xclip",
        &["-selection", "clipboard", "-t", "image/png", "-o"],
    )?;
    Ok(STANDARD.encode(&data))
}

fn read_image_macos() -> anyhow::Result<String> {
    let temp_path = std::env::temp_dir().join("opencode_clipboard_image.png");
    let temp_str = temp_path.to_str().context("temp path is not valid UTF-8")?;

    let script = format!(
        concat!(
            "set imageData to the clipboard as «class PNGf»\n",
            "set filePath to POSIX file \"{}\"\n",
            "set fileRef to open for access filePath with write permission\n",
            "set eof fileRef to 0\n",
            "write imageData to fileRef\n",
            "close access fileRef"
        ),
        temp_str
    );

    let output = Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .context("failed to execute osascript for clipboard image")?;

    if !output.status.success() {
        anyhow::bail!(
            "osascript clipboard image read failed with status {}",
            output.status
        );
    }

    let png_bytes =
        std::fs::read(&temp_path).context("failed to read clipboard image temp file")?;
    let _ = std::fs::remove_file(&temp_path);

    if png_bytes.is_empty() {
        anyhow::bail!("clipboard image temp file was empty");
    }

    Ok(STANDARD.encode(&png_bytes))
}

fn read_image_windows() -> anyhow::Result<String> {
    let ps_script = concat!(
        "Add-Type -AssemblyName System.Windows.Forms; ",
        "$img = [System.Windows.Forms.Clipboard]::GetImage(); ",
        "if ($img -eq $null) { exit 1 }; ",
        "$ms = New-Object System.IO.MemoryStream; ",
        "$img.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png); ",
        "[Convert]::ToBase64String($ms.ToArray())"
    );

    let output = Command::new("powershell")
        .args(&["-NoProfile", "-Command", ps_script])
        .output()
        .context("failed to execute PowerShell for clipboard image")?;

    if !output.status.success() {
        anyhow::bail!(
            "PowerShell clipboard image read failed with status {}",
            output.status
        );
    }

    let b64 = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if b64.is_empty() {
        anyhow::bail!("PowerShell clipboard image read returned empty output");
    }

    Ok(b64)
}
