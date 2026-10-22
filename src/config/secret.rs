use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{anyhow, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;

const ENVELOPE_PREFIX: &str = "enc:v1:";

/// Machine-local AES-256-GCM master key. Secrets in config are stored as
/// `enc:v1:<nonce>:<ciphertext>` and never written in plaintext.
pub struct MasterKey {
    key: [u8; 32],
}

impl MasterKey {
    pub fn load_or_create(path: &Path) -> Result<Self> {
        if path.exists() {
            let mut buf = Vec::new();
            fs::File::open(path)
                .with_context(|| format!("无法读取主密钥文件 {}", path.display()))?
                .read_to_end(&mut buf)?;
            let decoded = if buf.len() == 32 {
                buf
            } else {
                // Accept a base64-encoded key as well.
                URL_SAFE_NO_PAD
                    .decode(String::from_utf8_lossy(&buf).trim())
                    .unwrap_or_default()
            };
            if decoded.len() != 32 {
                return Err(anyhow!("主密钥文件已损坏（期望 32 字节）"));
            }
            let mut key = [0u8; 32];
            key.copy_from_slice(&decoded);
            return Ok(Self { key });
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut key = [0u8; 32];
        OsRng.fill_bytes(&mut key);
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut file = opts
            .open(path)
            .with_context(|| format!("无法创建主密钥文件 {}", path.display()))?;
        file.write_all(&key)?;
        file.flush()?;
        Ok(Self { key })
    }

    pub fn encrypt(&self, plaintext: &str) -> Result<String> {
        if plaintext.is_empty() {
            return Ok(String::new());
        }
        let cipher =
            Aes256Gcm::new_from_slice(&self.key).map_err(|_| anyhow!("初始化加密器失败"))?;
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, plaintext.as_bytes())
            .map_err(|_| anyhow!("加密失败"))?;
        Ok(format!(
            "{ENVELOPE_PREFIX}{}:{}",
            URL_SAFE_NO_PAD.encode(nonce_bytes),
            URL_SAFE_NO_PAD.encode(ciphertext)
        ))
    }

    pub fn decrypt(&self, envelope: &str) -> Result<String> {
        if envelope.is_empty() {
            return Ok(String::new());
        }
        // Legacy plaintext values are tolerated during migration.
        let Some(rest) = envelope.strip_prefix(ENVELOPE_PREFIX) else {
            return Ok(envelope.to_string());
        };
        let mut parts = rest.splitn(2, ':');
        let nonce_b64 = parts.next().ok_or_else(|| anyhow!("密文格式无效"))?;
        let ct_b64 = parts.next().ok_or_else(|| anyhow!("密文格式无效"))?;
        let nonce_bytes = URL_SAFE_NO_PAD
            .decode(nonce_b64)
            .map_err(|_| anyhow!("密文 nonce 无效"))?;
        if nonce_bytes.len() != 12 {
            return Err(anyhow!("密文 nonce 长度无效"));
        }
        let ciphertext = URL_SAFE_NO_PAD
            .decode(ct_b64)
            .map_err(|_| anyhow!("密文内容无效"))?;
        let cipher =
            Aes256Gcm::new_from_slice(&self.key).map_err(|_| anyhow!("初始化加密器失败"))?;
        let plaintext = cipher
            .decrypt(Nonce::from_slice(&nonce_bytes), ciphertext.as_ref())
            .map_err(|_| anyhow!("解密失败（主密钥不匹配或数据被篡改）"))?;
        String::from_utf8(plaintext).map_err(|_| anyhow!("解密结果不是有效 UTF-8"))
    }

    pub fn encrypt_if_needed(&self, value: &str) -> Result<String> {
        if value.is_empty() || value.starts_with(ENVELOPE_PREFIX) {
            Ok(value.to_string())
        } else {
            self.encrypt(value)
        }
    }
}

pub fn is_encrypted(value: &str) -> bool {
    value.starts_with(ENVELOPE_PREFIX)
}

pub fn generate_local_key() -> String {
    let mut bytes = [0u8; 24];
    OsRng.fill_bytes(&mut bytes);
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("yr-{hex}")
}

pub struct Paths {
    pub config_dir: PathBuf,
    pub config_file: PathBuf,
    pub db_file: PathBuf,
    pub master_key_file: PathBuf,
    pub exports_dir: PathBuf,
}

impl Paths {
    pub fn discover() -> Result<Self> {
        let root = if let Some(custom) = std::env::var_os("YOYO_RELAY_HOME") {
            std::path::PathBuf::from(custom)
        } else {
            dirs::config_dir()
                .or_else(dirs::data_local_dir)
                .ok_or_else(|| anyhow!("无法定位用户配置目录"))?
                .join("YoyoRelay")
        };
        fs::create_dir_all(&root)?;
        let exports_dir = root.join("exports");
        let _ = fs::create_dir_all(&exports_dir);
        Ok(Self {
            config_file: root.join("config.json"),
            db_file: root.join("usage.db"),
            master_key_file: root.join("master.key"),
            exports_dir,
            config_dir: root,
        })
    }

    #[cfg(test)]
    pub fn for_test(root: PathBuf) -> Self {
        Self {
            config_file: root.join("config.json"),
            db_file: root.join("usage.db"),
            master_key_file: root.join("master.key"),
            exports_dir: root.join("exports"),
            config_dir: root,
        }
    }
}
