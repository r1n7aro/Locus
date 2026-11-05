//! Failure-only evidence. Never starts Claude, sends credentials, changes routes,
//! performs a throughput test, or participates in retry/error classification.
use super::{bounded, timestamp, ProbeCheck, ProbeReport, SshConfig};
use serde_json::Value;
use std::{net::IpAddr, process::Stdio, sync::OnceLock, time::{Duration, Instant}};
use tokio::{io::AsyncReadExt, process::Command};

pub(super) async fn run(config: &SshConfig, failure_id: u64) -> ProbeReport {
    static GATE: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
    let mut report = ProbeReport { failure_id, at: timestamp(), state: "complete", checks: Vec::new() };
    let Ok(_permit) = GATE.get_or_init(|| tokio::sync::Semaphore::new(1)).try_acquire() else {
        report.state = "busy";
        return report;
    };
    if tokio::time::timeout(Duration::from_secs(10), collect(config, &mut report.checks)).await.is_err() {
        report.state = "timedOut";
    }
    report
}

fn check(name: &'static str, outcome: &'static str, started: Instant, detail: impl AsRef<str>) -> ProbeCheck {
    ProbeCheck { name, outcome, elapsed_ms: started.elapsed().as_millis() as u64, detail: bounded(detail.as_ref(), 700) }
}

async fn collect(config: &SshConfig, checks: &mut Vec<ProbeCheck>) {
    let started = Instant::now();
    // Same executable lookup, environment and options as the real transport,
    // not the user's Git Bash ssh. -G evaluates configuration without opening
    // another SSH session. Do not expose its full output (identity/proxy data).
    let mut command = super::super::configured_command(config);
    command.arg("-G").arg("--").arg(&config.target);
    let effective = match capture(command, Duration::from_secs(2)).await {
        Ok((stdout, _)) => match Effective::parse(&String::from_utf8_lossy(&stdout)) {
            Some(effective) => effective,
            None => { checks.push(check("sshConfig", "unavailable", started, "Could not read effective SSH hostname and port.")); return; }
        },
        Err(outcome) => { checks.push(check("sshConfig", outcome, started, "Effective SSH configuration probe did not complete; no alternate route was guessed.")); return; }
    };
    checks.push(check("sshConfig", "ok", started, format!("{}:{}; jump/proxy: {}", effective.host, effective.port, effective.proxied)));
    if effective.proxied {
        checks.push(check("tcp", "skipped", Instant::now(), "SSH uses ProxyJump/ProxyCommand; a direct TCP probe would not test that route."));
        return;
    }
    // Both checks are independent and bounded. A blocked/absent Tailscale CLI
    // must not obscure the TCP verdict, and neither blocks conversation recovery.
    let (tcp, tailscale) = tokio::join!(tcp_check(&effective), tailscale_check(&effective));
    checks.extend(tcp);
    checks.extend(tailscale);
}

struct Effective { host: String, port: u16, proxied: bool }
impl Effective {
    fn parse(text: &str) -> Option<Self> {
        let mut host = None;
        let mut port = None;
        let mut proxied = false;
        for line in text.lines() {
            let Some((key, value)) = line.split_once(' ') else { continue; };
            let value = value.trim();
            match key {
                "hostname" if !value.is_empty() && value.len() <= 253 => host = Some(value.to_owned()),
                "port" => port = value.parse::<u16>().ok().filter(|port| *port > 0),
                "proxycommand" | "proxyjump" => proxied |= !value.is_empty() && value != "none",
                _ => {},
            }
        }
        Some(Self { host: host?, port: port?, proxied })
    }
}

async fn tcp_check(target: &Effective) -> Vec<ProbeCheck> {
    let started = Instant::now();
    let mut stream = match tokio::time::timeout(Duration::from_secs(3), tokio::net::TcpStream::connect((target.host.as_str(), target.port))).await {
        Ok(Ok(stream)) => stream,
        Ok(Err(error)) => return vec![check("tcp", "failed", started, format!("TCP connect: {:?}", error.kind()))],
        Err(_) => return vec![check("tcp", "timedOut", started, "TCP connection did not complete within 3s.")],
    };
    let mut checks = vec![check("tcp", "ok", started, "TCP connection established; this does not prove SSH authentication or helper health.")];
    let started = Instant::now();
    let read = async {
        let mut bytes = Vec::new();
        while bytes.len() < 256 {
            let byte = stream.read_u8().await?;
            bytes.push(byte);
            if byte == b'\n' { break; }
        }
        Ok::<_, std::io::Error>(bytes)
    };
    checks.push(match tokio::time::timeout(Duration::from_secs(2), read).await {
        Ok(Ok(bytes)) if bytes.starts_with(b"SSH-") => check("sshBanner", "ok", started, String::from_utf8_lossy(&bytes).trim()),
        Ok(Ok(_)) => check("sshBanner", "unknown", started, "Received a non-SSH identification line; no authentication was attempted."),
        Ok(Err(error)) => check("sshBanner", "failed", started, format!("SSH identification read: {:?}", error.kind())),
        Err(_) => check("sshBanner", "timedOut", started, "TCP connected but no SSH identification line arrived within 2s."),
    });
    checks
}

async fn tailscale_check(target: &Effective) -> Vec<ProbeCheck> {
    let Ok(ip) = target.host.trim_matches(['[', ']']).parse::<IpAddr>() else { return Vec::new(); };
    if !is_tailnet(ip) { return Vec::new(); }
    let started = Instant::now();
    let mut command = tailscale();
    command.args(["status", "--json"]);
    let (stdout, _) = match capture(command, Duration::from_secs(2)).await {
        Ok(output) => output,
        Err(outcome) => return vec![check("tailscale", outcome, started, "Tailscale status is unavailable; this does not establish a network failure.")],
    };
    let status = match serde_json::from_slice::<Value>(&stdout) {
        Ok(value) => value,
        Err(_) => return vec![check("tailscale", "unknown", started, "Could not parse Tailscale status.")],
    };
    let Some(detail) = peer_detail(&status, &ip.to_string()) else {
        return vec![check("tailscale", "unknown", started, "Target was not present in the local Tailscale peer snapshot.")];
    };
    let mut checks = vec![check("tailscale", "ok", started, detail)];
    let started = Instant::now();
    let mut command = tailscale();
    command.args(["ping", "--c=1", "--timeout=2s", "--until-direct=false", &ip.to_string()]);
    checks.push(match capture(command, Duration::from_secs(3)).await {
        Ok((stdout, _)) => check("tailscalePing", "ok", started, String::from_utf8_lossy(&stdout)),
        Err(outcome) => check("tailscalePing", outcome, started, "The single Tailscale-layer probe did not succeed. It is not an SSH/authentication test."),
    });
    checks
}

fn is_tailnet(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => { let octets = ip.octets(); octets[0] == 100 && (64..=127).contains(&octets[1]) },
        IpAddr::V6(ip) => ip.segments()[..3] == [0xfd7a, 0x115c, 0xa1e0],
    }
}
fn peer_detail(status: &Value, ip: &str) -> Option<String> {
    let peer = status["Peer"].as_object()?.values().find(|peer|
        peer["TailscaleIPs"].as_array().is_some_and(|ips| ips.iter().any(|value| value.as_str() == Some(ip))))?;
    let address = peer["CurAddr"].as_str().unwrap_or("");
    // Relay is also a preferred-region field. Its mere presence is NOT evidence
    // that the current data path is relayed. Describe the observed fields only.
    Some(format!("backend={}; online={}; active={}; endpoint={}; relay region={}. A direct endpoint does not rule out a local VPN/proxy.",
        status["BackendState"].as_str().unwrap_or("unknown"), peer["Online"], peer["Active"],
        if address.is_empty() { "unknown" } else { address }, peer["Relay"].as_str().unwrap_or("unknown")))
}
fn tailscale() -> Command {
    #[cfg(windows)]
    if let Some(program_files) = std::env::var_os("ProgramFiles") {
        let path = std::path::PathBuf::from(program_files).join("Tailscale/tailscale.exe");
        if path.is_file() { return Command::new(path); }
    }
    Command::new("tailscale")
}

async fn capture(mut command: Command, timeout: Duration) -> Result<(Vec<u8>, Vec<u8>), &'static str> {
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(super::super::CREATE_NO_WINDOW);
    let mut child = command.spawn().map_err(|_| "unavailable")?;
    #[cfg(windows)]
    let _job = super::super::pty_job::PtyJob::attach(child.id().ok_or("unavailable")?).map_err(|_| "unavailable")?;
    let mut stdout = child.stdout.take().ok_or("unavailable")?.take(256 * 1024 + 1);
    let mut stderr = child.stderr.take().ok_or("unavailable")?.take(8193);
    let result = tokio::time::timeout(timeout, async {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let (a, b, status) = tokio::join!(stdout.read_to_end(&mut out), stderr.read_to_end(&mut err), child.wait());
        a.map_err(|_| "failed")?;
        b.map_err(|_| "failed")?;
        if out.len() > 256 * 1024 || err.len() > 8192 { return Err("limited"); }
        if !status.map_err(|_| "failed")?.success() { return Err("failed"); }
        Ok((out, err))
    }).await;
    result.unwrap_or(Err("timedOut"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn effective_config_does_not_guess_a_direct_route_through_a_jump_host() {
        let parsed = Effective::parse("hostname host.example\nport 2222\nproxyjump user@gateway\nidentityfile secret\n").unwrap();
        assert_eq!((parsed.host.as_str(), parsed.port, parsed.proxied), ("host.example", 2222, true));
        assert!(Effective::parse("hostname host\nport 0").is_none());
        assert!(!Effective::parse("hostname 127.0.0.1\nport 22\nproxycommand none").unwrap().proxied);
    }
    #[test]
    fn tailnet_probe_is_scoped_and_never_returns_other_peers_or_keys() {
        assert!(is_tailnet("100.101.102.103".parse().unwrap()));
        assert!(!is_tailnet("100.128.0.1".parse().unwrap()));
        assert!(!is_tailnet("192.168.0.1".parse().unwrap()));
        let detail = peer_detail(&json!({"BackendState":"Running", "PrivateKey":"secret", "Peer": {
            "other":{"TailscaleIPs":["100.64.0.2"],"HostName":"private-other-host"},
            "wanted":{"TailscaleIPs":["100.64.0.1"],"Online":true,"Active":true,"CurAddr":"1.2.3.4:1234","Relay":"nyc"}
        }}), "100.64.0.1").unwrap();
        assert!(detail.contains("1.2.3.4:1234") && detail.contains("relay region=nyc"));
        assert!(!detail.contains("secret") && !detail.contains("private-other-host"));
    }
    #[tokio::test]
    async fn tcp_banner_check_does_not_send_authentication_or_payload() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            let (mut stream, _) = listener.accept().await.unwrap();
            stream.write_all(b"SSH-2.0-test\r\n").await.unwrap();
            let mut byte = [0];
            assert_eq!(stream.read(&mut byte).await.unwrap(), 0);
        });
        let checks = tcp_check(&Effective { host: "127.0.0.1".into(), port, proxied: false }).await;
        assert!(checks.iter().all(|c| c.outcome == "ok"));
        assert_eq!(checks[1].detail, "SSH-2.0-test");
        server.await.unwrap();
    }
}
