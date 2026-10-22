//! Best-effort per-user autostart registration.
//!
//! - Windows: HKCU\Software\Microsoft\Windows\CurrentVersion\Run
//! - macOS: ~/Library/LaunchAgents/com.yoyorelay.app.plist
//! - Linux: ~/.config/autostart/yoyo-relay.desktop

use anyhow::{anyhow, Context, Result};

pub fn set_enabled(enabled: bool) -> Result<()> {
    let exe = std::env::current_exe().context("无法定位 YoyoRelay 可执行文件路径")?;
    imp::set_enabled(enabled, &exe)
}

#[allow(dead_code)]
pub fn is_enabled() -> bool {
    imp::is_enabled()
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE_NAME: &str = "YoyoRelay";
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    fn reg() -> Command {
        let mut command = Command::new("reg.exe");
        command.creation_flags(CREATE_NO_WINDOW);
        command
    }

    pub fn set_enabled(enabled: bool, exe: &std::path::Path) -> Result<()> {
        if enabled {
            let value = format!("\"{}\"", exe.display());
            let status = reg()
                .args([
                    "add", RUN_KEY, "/v", VALUE_NAME, "/t", "REG_SZ", "/d", &value, "/f",
                ])
                .status()
                .context("无法调用 reg.exe（开机自启）")?;
            if !status.success() {
                return Err(anyhow!(
                    "写入注册表启动项失败（reg.exe 退出码 {:?}）",
                    status.code()
                ));
            }
        } else {
            let status = reg()
                .args(["delete", RUN_KEY, "/v", VALUE_NAME, "/f"])
                .status()
                .context("无法调用 reg.exe（开机自启）")?;
            // Deleting a missing value is not an error for our purposes.
            let _ = status;
        }
        Ok(())
    }

    pub fn is_enabled() -> bool {
        reg()
            .args(["query", RUN_KEY, "/v", VALUE_NAME])
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;

    fn plist_path() -> Result<PathBuf> {
        let home = dirs::home_dir().ok_or_else(|| anyhow!("无法定位用户主目录"))?;
        Ok(home
            .join("Library")
            .join("LaunchAgents")
            .join("com.yoyorelay.app.plist"))
    }

    fn xml_escape(value: &str) -> String {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }

    pub fn set_enabled(enabled: bool, exe: &std::path::Path) -> Result<()> {
        let path = plist_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if enabled {
            let content = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>com.yoyorelay.app</string>
  <key>ProgramArguments</key>
  <array>
    <string>{}</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>ProcessType</key><string>Interactive</string>
</dict>
</plist>
"#,
                xml_escape(&exe.display().to_string())
            );
            fs::write(&path, content).context("写入 LaunchAgent 失败")?;
            let _ = Command::new("launchctl")
                .args(["load", "-w"])
                .arg(&path)
                .status();
        } else {
            let _ = Command::new("launchctl")
                .args(["unload", "-w"])
                .arg(&path)
                .status();
            if path.exists() {
                fs::remove_file(&path).context("删除 LaunchAgent 失败")?;
            }
        }
        Ok(())
    }

    pub fn is_enabled() -> bool {
        plist_path().map(|path| path.exists()).unwrap_or(false)
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn desktop_path() -> Result<PathBuf> {
        let config = dirs::config_dir().ok_or_else(|| anyhow!("无法定位用户配置目录"))?;
        Ok(config.join("autostart").join("yoyo-relay.desktop"))
    }

    fn desktop_exec(exe: &std::path::Path) -> String {
        let raw = exe.display().to_string();
        if raw.contains(' ') || raw.contains('\t') {
            format!("\"{raw}\"")
        } else {
            raw
        }
    }

    pub fn set_enabled(enabled: bool, exe: &std::path::Path) -> Result<()> {
        let path = desktop_path()?;
        if enabled {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let content = format!(
                "[Desktop Entry]\nType=Application\nName=YoyoRelay\nComment=Local LLM relay and failover proxy\nExec={}\nTerminal=false\nX-GNOME-Autostart-enabled=true\nHidden=false\n",
                desktop_exec(exe)
            );
            fs::write(&path, content).context("写入 XDG autostart 文件失败")?;
        } else if path.exists() {
            fs::remove_file(&path).context("删除 XDG autostart 文件失败")?;
        }
        Ok(())
    }

    pub fn is_enabled() -> bool {
        desktop_path().map(|path| path.exists()).unwrap_or(false)
    }
}
