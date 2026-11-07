//! HTTP access behind a small trait so the engine can be tested with a fake server.
use std::io::Read;
use std::path::Path;
use std::time::Duration;

pub trait Http {
    fn get(&self, url: &str) -> Result<Vec<u8>, String>;
    fn post_form(&self, url: &str, fields: &[(&str, &str)]) -> Result<Vec<u8>, String>;
    fn download(&self, url: &str, dest: &Path) -> Result<(), String>;
}

pub struct Real {
    agent: ureq::Agent,
}

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) TierSDriverManage";

impl Real {
    pub fn new() -> Real {
        let tls = native_tls::TlsConnector::new().expect("TLS");
        Real {
            agent: ureq::AgentBuilder::new()
                .tls_connector(std::sync::Arc::new(tls))
                .timeout_connect(Duration::from_secs(20))
                .timeout_read(Duration::from_secs(120))
                .redirects(8)
                .user_agent(UA)
                .build(),
        }
    }
}

impl Default for Real { fn default() -> Self { Self::new() } }

fn read_all(resp: ureq::Response) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    resp.into_reader().take(512 * 1024 * 1024).read_to_end(&mut buf).map_err(|e| e.to_string())?;
    Ok(buf)
}

impl Http for Real {
    fn get(&self, url: &str) -> Result<Vec<u8>, String> {
        read_all(self.agent.get(url).call().map_err(|e| e.to_string())?)
    }
    fn post_form(&self, url: &str, fields: &[(&str, &str)]) -> Result<Vec<u8>, String> {
        read_all(self.agent.post(url).send_form(fields).map_err(|e| e.to_string())?)
    }
    fn download(&self, url: &str, dest: &Path) -> Result<(), String> {
        let resp = self.agent.get(url).timeout_read_fix().call().map_err(|e| e.to_string())?;
        let mut out = std::fs::File::create(dest).map_err(|e| e.to_string())?;
        std::io::copy(&mut resp.into_reader(), &mut out).map_err(|e| e.to_string())?;
        Ok(())
    }
}

trait ReadTimeoutFix { fn timeout_read_fix(self) -> Self; }
impl ReadTimeoutFix for ureq::Request {
    // big driver files can pause for a long time between chunks
    fn timeout_read_fix(self) -> Self { self.timeout(Duration::from_secs(3600)) }
}

/// Test double: answers by URL substring.
#[cfg(test)]
pub struct Fake { pub routes: Vec<(String, Vec<u8>)>, pub calls: std::cell::RefCell<Vec<String>> }
#[cfg(test)]
impl Fake {
    pub fn new(routes: Vec<(&str, &str)>) -> Fake {
        Fake { routes: routes.into_iter().map(|(k, v)| (k.to_string(), v.as_bytes().to_vec())).collect(), calls: Default::default() }
    }
    fn find(&self, url: &str) -> Result<Vec<u8>, String> {
        self.calls.borrow_mut().push(url.to_string());
        self.routes.iter().find(|(k, _)| url.contains(k.as_str())).map(|(_, v)| v.clone()).ok_or_else(|| format!("unexpected url {url}"))
    }
}
#[cfg(test)]
impl Http for Fake {
    fn get(&self, url: &str) -> Result<Vec<u8>, String> { self.find(url) }
    fn post_form(&self, url: &str, _f: &[(&str, &str)]) -> Result<Vec<u8>, String> { self.find(url) }
    fn download(&self, url: &str, dest: &Path) -> Result<(), String> { std::fs::write(dest, self.find(url)?).map_err(|e| e.to_string()) }
}
