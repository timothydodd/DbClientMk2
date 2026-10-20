//! SSH tunnel: a local 127.0.0.1 listener forwarded to the database port as seen from the SSH
//! server, over `direct-tcpip` channels. Host keys are checked against `~/.dbclient/known_hosts`
//! (`host:port keytype base64`) and `~/.ssh/known_hosts` (OpenSSH format); an unknown key is put
//! to the user (trust-on-first-use) and pinned when accepted. A changed key is refused.

use crate::{DataError, HostKeyInfo, HostKeyPrompt, Result};
use dbclient_core::models::ConnectionConfig;
use russh::client::{self, Handle};
use russh::keys::{self, HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

fn err(e: impl std::fmt::Display) -> DataError {
    DataError::Ssh(e.to_string())
}

#[derive(Debug, Clone)]
pub struct TunnelSettings {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub key_file: String,
    pub key_passphrase: String,
    /// Database port on the remote side (reached as 127.0.0.1:<port> from the SSH server).
    pub remote_port: u16,
}

impl TunnelSettings {
    /// The SSH port is the connection's `port` field (the database listens on `ssh_remote_port`).
    pub fn from_config(c: &ConnectionConfig) -> Result<Self> {
        let port: u16 = c
            .port
            .trim()
            .parse()
            .ok()
            .filter(|p| *p > 0)
            .ok_or_else(|| DataError::Config(format!("Invalid SSH port: '{}'. Port must be a number between 1 and 65535.", c.port)))?;
        Ok(Self {
            host: c.ssh_host.clone(),
            port,
            user: c.ssh_user.clone(),
            password: c.ssh_password.clone(),
            key_file: c.ssh_key_file.clone(),
            key_passphrase: c.ssh_key_passphrase.clone(),
            remote_port: c.ssh_remote_port,
        })
    }
}

/// Keeps one tunnel up across queries, reconnecting when it dropped.
pub struct TunnelManager {
    settings: TunnelSettings,
    prompt: Arc<dyn HostKeyPrompt>,
    known_hosts: PathBuf,
    tunnel: Mutex<Option<Tunnel>>,
}

impl TunnelManager {
    pub fn new(settings: TunnelSettings, prompt: Arc<dyn HostKeyPrompt>, known_hosts: PathBuf) -> Self {
        Self { settings, prompt, known_hosts, tunnel: Mutex::new(None) }
    }

    /// Local port the database is reachable on; brings the tunnel up when needed.
    pub async fn local_port(&self) -> Result<u16> {
        let mut guard = self.tunnel.lock().await;
        if let Some(t) = guard.as_ref() {
            if t.is_alive() {
                return Ok(t.local_port);
            }
        }
        *guard = None;
        let session = Session::connect(&self.settings, Arc::clone(&self.prompt), &self.known_hosts).await?;
        let tunnel = session.tunnel("127.0.0.1".into(), self.settings.remote_port).await?;
        let port = tunnel.local_port;
        *guard = Some(tunnel);
        Ok(port)
    }

    pub async fn close(&self) {
        *self.tunnel.lock().await = None;
    }
}

struct Verifier {
    host: String,
    port: u16,
    prompt: Arc<dyn HostKeyPrompt>,
    known_hosts: PathBuf,
    /// Set when the key was refused, so the connect error can say why.
    refusal: Arc<std::sync::Mutex<Option<String>>>,
}

impl client::Handler for Verifier {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> std::result::Result<bool, Self::Error> {
        let public = match key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.clone(),
            PublicKeyOrCertificate::Certificate(cert) => cert.public_key().clone().into(),
        };
        let key_type = public.algorithm().to_string();
        let key_bytes = public.to_bytes().map_err(|_| russh::Error::CouldNotReadKey)?;
        let key_b64 = base64_encode(&key_bytes);
        let sha256 = public.fingerprint(HashAlg::Sha256).to_string();
        let md5 = format!("MD5:{}", md5_hex(&key_bytes));

        let stored = find_stored_keys(&self.known_hosts, &self.host, self.port, &key_type);
        if !stored.is_empty() {
            if stored.iter().any(|k| k == &key_b64) {
                return Ok(true);
            }
            *self.refusal.lock().unwrap() = Some(format!(
                "HOST KEY CHANGED for {}:{} ({key_type}). The key presented ({sha256}) does not match the stored key. \
                 This could indicate a man-in-the-middle attack. If the server key was legitimately changed, remove the entry \
                 for {}:{} from {} and try again.",
                self.host,
                self.port,
                self.host,
                self.port,
                self.known_hosts.display()
            ));
            return Ok(false);
        }

        let info = HostKeyInfo {
            host: self.host.clone(),
            port: self.port,
            key_type: key_type.clone(),
            fingerprint_sha256: sha256.clone(),
            fingerprint_md5: md5,
        };
        if self.prompt.confirm(info).await {
            append_known_host(&self.known_hosts, &self.host, self.port, &key_type, &key_b64);
            Ok(true)
        } else {
            *self.refusal.lock().unwrap() =
                Some(format!("SSH host key for {}:{} ({key_type}, {sha256}) was rejected.", self.host, self.port));
            Ok(false)
        }
    }
}

pub struct Session {
    handle: Arc<Handle<Verifier>>,
}

impl Session {
    pub async fn connect(cfg: &TunnelSettings, prompt: Arc<dyn HostKeyPrompt>, known_hosts: &Path) -> Result<Session> {
        if cfg.host.trim().is_empty() {
            return Err(DataError::Config("SSH host is empty.".into()));
        }
        let config = Arc::new(client::Config {
            keepalive_interval: Some(Duration::from_secs(30)),
            keepalive_max: 3,
            inactivity_timeout: None,
            ..Default::default()
        });
        let refusal = Arc::new(std::sync::Mutex::new(None));
        let verifier = Verifier {
            host: cfg.host.clone(),
            port: cfg.port,
            prompt,
            known_hosts: known_hosts.to_path_buf(),
            refusal: Arc::clone(&refusal),
        };
        // Generous: the user may be looking at a host-key prompt.
        let connecting = client::connect(config, (cfg.host.as_str(), cfg.port), verifier);
        let result = tokio::time::timeout(Duration::from_secs(300), connecting)
            .await
            .map_err(|_| err(format!("timed out connecting to {}:{}", cfg.host, cfg.port)))?;
        let mut handle = match result {
            Ok(h) => h,
            Err(e) => {
                if let Some(reason) = refusal.lock().unwrap().clone() {
                    return Err(DataError::Ssh(reason));
                }
                return Err(err(e));
            }
        };

        let auth = if !cfg.key_file.trim().is_empty() {
            let path = expand_home(cfg.key_file.trim());
            let passphrase = (!cfg.key_passphrase.is_empty()).then_some(cfg.key_passphrase.as_str());
            let key = keys::load_secret_key(&path, passphrase).map_err(|e| err(format!("can't load key {path}: {e}")))?;
            let hash = handle.best_supported_rsa_hash().await.map_err(err)?.flatten();
            handle.authenticate_publickey(&cfg.user, PrivateKeyWithHashAlg::new(Arc::new(key), hash)).await.map_err(err)?
        } else {
            handle.authenticate_password(&cfg.user, &cfg.password).await.map_err(err)?
        };
        if !auth.success() {
            return Err(err(format!("authentication failed for {}@{}", cfg.user, cfg.host)));
        }
        Ok(Session { handle: Arc::new(handle) })
    }

    /// Listen on 127.0.0.1:<random> and forward each connection to `remote_host:remote_port`.
    pub async fn tunnel(self, remote_host: String, remote_port: u16) -> Result<Tunnel> {
        let listener = TcpListener::bind("127.0.0.1:0").await.map_err(err)?;
        let local_port = listener.local_addr().map_err(err)?.port();
        let handle = Arc::clone(&self.handle);
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut socket, peer)) = listener.accept().await else { break };
                let handle = Arc::clone(&handle);
                let host = remote_host.clone();
                tokio::spawn(async move {
                    let channel = match handle.channel_open_direct_tcpip(host, remote_port as u32, peer.ip().to_string(), peer.port() as u32).await {
                        Ok(c) => c,
                        Err(_) => return,
                    };
                    let mut stream = channel.into_stream();
                    let _ = tokio::io::copy_bidirectional(&mut socket, &mut stream).await;
                });
            }
        });
        Ok(Tunnel { local_port, task, session: self })
    }
}

/// Keeps the SSH session and forwarding task alive; dropping it tears both down.
pub struct Tunnel {
    pub local_port: u16,
    task: JoinHandle<()>,
    session: Session,
}

impl Tunnel {
    pub fn is_alive(&self) -> bool {
        !self.task.is_finished() && !self.session.handle.is_closed()
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn expand_home(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
        return format!("{home}/{rest}");
    }
    path.to_string()
}

fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn md5_hex(bytes: &[u8]) -> String {
    use md5::Digest;
    let digest = md5::Md5::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":")
}

fn user_ssh_known_hosts() -> PathBuf {
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
    Path::new(&home).join(".ssh").join("known_hosts")
}

/// All stored keys (base64) for host/port/key type, from our file and the user's OpenSSH file.
fn find_stored_keys(app_known_hosts: &Path, host: &str, port: u16, key_type: &str) -> Vec<String> {
    let mut keys = Vec::new();
    read_known_hosts(app_known_hosts, host, port, key_type, &mut keys);
    read_known_hosts(&user_ssh_known_hosts(), host, port, key_type, &mut keys);
    keys
}

pub(crate) fn read_known_hosts(path: &Path, host: &str, port: u16, key_type: &str, keys: &mut Vec<String>) {
    let Ok(text) = std::fs::read_to_string(path) else { return };
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('@') {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 3 {
            continue;
        }
        if parts[0].starts_with("|1|") {
            continue; // hashed entry
        }
        if !parts[1].eq_ignore_ascii_case(key_type) {
            continue;
        }
        if parts[0].split(',').any(|pattern| host_matches(pattern, host, port)) && !keys.iter().any(|k| k == parts[2]) {
            keys.push(parts[2].to_string());
        }
    }
}

/// dbclient format `host:port`; OpenSSH `host` (port 22) or `[host]:port`.
pub(crate) fn host_matches(pattern: &str, host: &str, port: u16) -> bool {
    if let Some(rest) = pattern.strip_prefix('[') {
        let Some(close) = rest.find(']') else { return false };
        let h = &rest[..close];
        let Some(p) = rest[close + 1..].strip_prefix(':').and_then(|p| p.parse::<u16>().ok()) else { return false };
        return p == port && h.eq_ignore_ascii_case(host);
    }
    if let Some(idx) = pattern.rfind(':') {
        if let Ok(p) = pattern[idx + 1..].parse::<u16>() {
            return p == port && pattern[..idx].eq_ignore_ascii_case(host);
        }
    }
    port == 22 && pattern.eq_ignore_ascii_case(host)
}

fn append_known_host(path: &Path, host: &str, port: u16, key_type: &str, key_b64: &str) {
    // Best effort: the connection is trusted for this session regardless.
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let needs_newline = std::fs::read_to_string(path).map(|t| !t.is_empty() && !t.ends_with('\n')).unwrap_or(false);
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = write!(f, "{}{host}:{port} {key_type} {key_b64}\n", if needs_newline { "\n" } else { "" });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_patterns() {
        assert!(host_matches("db.example", "db.example", 22));
        assert!(!host_matches("db.example", "db.example", 2222));
        assert!(host_matches("[db.example]:2222", "DB.example", 2222));
        assert!(host_matches("db.example:2222", "db.example", 2222));
        assert!(!host_matches("other:2222", "db.example", 2222));
    }

    #[test]
    fn reads_both_formats_and_appends() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("known_hosts");
        std::fs::write(&p, "# comment\n|1|hashed abc def\nhost.a:22 ssh-ed25519 AAAA1\n[host.b]:2200 ssh-rsa BBBB\nhost.c ssh-ed25519 CCCC\n").unwrap();
        let mut keys = Vec::new();
        read_known_hosts(&p, "host.a", 22, "ssh-ed25519", &mut keys);
        assert_eq!(keys, ["AAAA1"]);
        keys.clear();
        read_known_hosts(&p, "host.b", 2200, "ssh-rsa", &mut keys);
        assert_eq!(keys, ["BBBB"]);
        keys.clear();
        read_known_hosts(&p, "host.c", 22, "ssh-ed25519", &mut keys);
        assert_eq!(keys, ["CCCC"]);
        append_known_host(&p, "host.d", 22, "ssh-ed25519", "DDDD");
        keys.clear();
        read_known_hosts(&p, "host.d", 22, "ssh-ed25519", &mut keys);
        assert_eq!(keys, ["DDDD"]);
    }

    #[test]
    fn settings_need_a_numeric_ssh_port() {
        let mut c = ConnectionConfig { use_ssh: true, port: "abc".into(), ..Default::default() };
        assert!(TunnelSettings::from_config(&c).is_err());
        c.port = "22".into();
        assert_eq!(TunnelSettings::from_config(&c).unwrap().port, 22);
    }
}
