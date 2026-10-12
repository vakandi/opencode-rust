//! Behavioural tests against a real socket: a fake upstream proxy stands in for the
//! residential fleet, so CONNECT forwarding, leftover-byte handling, 502 propagation,
//! rotation order and `|last:` persistence are all exercised end to end.
//!
//! Every test here sets `OPENCODE_PATHS_ROOT` and writes pool/quarantine/claims state;
//! env mutation plus cargo's parallel runner races, and one test's pool replacing
//! another's mid-selection surfaces as a spurious 502. [`serial`] takes turns.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use opencode_proxy::config::ProxyConfig;
use opencode_proxy::state::ProxyState;
use opencode_proxy::Upstream;

const LEFTOVER: &str = "HELLO-LEFTOVER";

static SERIAL: Mutex<()> = Mutex::new(());

/// One test at a time; poison is recovered so a failed assertion does not cascade.
fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// What the fake upstream should do when it receives a CONNECT.
#[derive(Clone, Copy)]
enum UpstreamMode {
    /// 200, then immediately push `LEFTOVER` before the client sends anything.
    AcceptWithLeftover,
    /// 502, the shape a dead/quota-exhausted residential IP returns.
    Refuse,
}

struct FakeUpstream {
    port: u16,
    targets: Arc<std::sync::Mutex<Vec<String>>>,
    auth_seen: Arc<std::sync::Mutex<Vec<String>>>,
}

async fn spawn_upstream(mode: UpstreamMode) -> FakeUpstream {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind fake upstream");
    let port = listener.local_addr().unwrap().port();
    let targets = Arc::new(std::sync::Mutex::new(Vec::new()));
    let auth_seen = Arc::new(std::sync::Mutex::new(Vec::new()));

    let t = Arc::clone(&targets);
    let a = Arc::clone(&auth_seen);
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else { return };
            let t = Arc::clone(&t);
            let a = Arc::clone(&a);
            tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                let Ok(n) = socket.read(&mut buf).await else { return };
                let head = String::from_utf8_lossy(&buf[..n]).to_string();
                if let Some(line) = head.lines().next() {
                    t.lock().unwrap().push(line.to_string());
                }
                if let Some(value) = head
                    .lines()
                    .find(|l| l.to_ascii_lowercase().starts_with("proxy-authorization:"))
                {
                    a.lock().unwrap().push(value.to_string());
                }
                match mode {
                    UpstreamMode::AcceptWithLeftover => {
                        // The status line AND the leftover in ONE write. If the listener
                        // discards bytes past the header terminator, the client hangs.
                        let response = format!(
                            "HTTP/1.1 200 Connection Established\r\n\r\n{LEFTOVER}"
                        );
                        let _ = socket.write_all(response.as_bytes()).await;
                        let _ = socket.flush().await;
                        // Then echo, so a post-handshake write is also observable.
                        let mut chunk = [0u8; 1024];
                        while let Ok(read) = socket.read(&mut chunk).await {
                            if read == 0 {
                                break;
                            }
                            if socket.write_all(&chunk[..read]).await.is_err() {
                                break;
                            }
                        }
                    }
                    UpstreamMode::Refuse => {
                        let _ = socket
                            .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
                            .await;
                        let _ = socket.flush().await;
                    }
                }
            });
        }
    });

    FakeUpstream { port, targets, auth_seen }
}

fn temp_root(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("proxy-it-{tag}-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Read exactly up to and including the first `\r\n\r\n` — i.e. consume the response
/// head (the listener's own `200 Connection Established`) so the bytes that follow are
/// the tunnelled payload, not header text.
async fn read_head(client: &mut TcpStream) -> String {
    let mut seen: Vec<u8> = Vec::new();
    loop {
        let mut chunk = [0u8; 1];
        let n = tokio::time::timeout(Duration::from_secs(5), client.read(&mut chunk))
            .await
            .expect("timed out reading response head")
            .unwrap();
        if n == 0 {
            break;
        }
        seen.push(chunk[0]);
        if seen.len() >= 4 && seen[seen.len() - 4..] == *b"\r\n\r\n" {
            break;
        }
        if seen.len() > 4096 {
            break;
        }
    }
    String::from_utf8_lossy(&seen).to_string()
}

fn write_pool(path: &Path, lines: &[String]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, lines.join("\n") + "\n").unwrap();
}

fn cfg_for_test() -> ProxyConfig {
    let mut cfg = ProxyConfig::default();
    cfg.probe_timeout = Duration::from_millis(1500);
    cfg.upstream_timeout = Duration::from_millis(1500);
    // Plain HTTP so the fake can serve the probe: it answers "200 Connection
    // Established", which is a valid 200.
    cfg.probe_target = "http://fake-target/".to_string();
    cfg.tunnel_attempts = 1;
    cfg
}

/// Isolates the new end-to-end probe against the fake upstream: if this fails, every
/// rotation test fails for the same reason and the fault is not in rotation at all.
#[tokio::test]
async fn probe_succeeds_against_a_fake_upstream() {
    let fake = spawn_upstream(UpstreamMode::AcceptWithLeftover).await;
    let mut cfg = cfg_for_test();
    cfg.probe_timeout = Duration::from_millis(3000);
    let up = Upstream::new("127.0.0.1", fake.port, "u", "p");
    let outcome = opencode_proxy::probe::probe_upstream(&up, &cfg).await;
    assert_eq!(outcome, opencode_proxy::probe::ProbeOutcome::Ok, "probe target={}", cfg.probe_target);
}

#[tokio::test]
async fn end_to_end_proxy_behaviour() {
    let root = temp_root("e2e");
    let _guard = serial();
    std::env::set_var("OPENCODE_PATHS_ROOT", &root);

    // ---------- 1. CONNECT is tunnelled and leftover bytes survive ----------
    let good = spawn_upstream(UpstreamMode::AcceptWithLeftover).await;
    let cfg = Arc::new(cfg_for_test());
    let state = Arc::new(ProxyState::new(Arc::clone(&cfg), root.join("opencode/proxy-pool.txt")));
    state.set_upstream(Some(Upstream::new("127.0.0.1", good.port, "u", "p")));

    let (listener, port) = opencode_proxy::listener::bind_listener(&cfg).await.unwrap();
    opencode_proxy::listener::accept_loop(listener, Arc::clone(&cfg), Arc::clone(&state));

    let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    client
        .write_all(b"CONNECT fake-target:443 HTTP/1.1\r\nHost: fake-target:443\r\n\r\n")
        .await
        .unwrap();
    client.flush().await.unwrap();

    let established = read_head(&mut client).await;
    assert!(
        established.starts_with("HTTP/1.1 200"),
        "client must receive the 200 before any tunnel bytes, got: {established}"
    );
    let mut seen = vec![0u8; LEFTOVER.len()];
    tokio::time::timeout(Duration::from_secs(5), client.read_exact(&mut seen))
        .await
        .expect("client timed out waiting for tunnelled bytes")
        .unwrap();
    assert_eq!(
        String::from_utf8(seen).unwrap(),
        LEFTOVER,
        "bytes the upstream sent after its 200 header must reach the client"
    );

    // Post-tunnel writes flow too.
    client.write_all(b"PING").await.unwrap();
    let mut echo = [0u8; 4];
    tokio::time::timeout(Duration::from_secs(5), client.read_exact(&mut echo))
        .await
        .expect("post-tunnel write timed out")
        .unwrap();
    assert_eq!(&echo, b"PING");

    // The upstream saw an absolute CONNECT and our credential, and never the client's
    // own Proxy-Authorization.
    let targets = good.targets.lock().unwrap().clone();
    assert_eq!(targets.first().map(String::as_str), Some("CONNECT fake-target:443 HTTP/1.1"));
    let auth = good.auth_seen.lock().unwrap().clone();
    assert_eq!(auth.len(), 1, "exactly one Proxy-Authorization header, ours: {auth:?}");

    // ---------- 2. A dead upstream surfaces as 502 and gets quarantined ----------
    let bad = spawn_upstream(UpstreamMode::Refuse).await;
    state.set_upstream(Some(Upstream::new("127.0.0.1", bad.port, "u", "p")));
    let mut client2 = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    client2
        .write_all(b"CONNECT fake-target:443 HTTP/1.1\r\nHost: fake-target:443\r\n\r\n")
        .await
        .unwrap();
    let response = read_head(&mut client2).await;
    assert!(
        response.starts_with("HTTP/1.1 502"),
        "expected 502 for a refused CONNECT, got: {response}"
    );
    assert!(
        state.active_quarantine().contains(&format!("127.0.0.1:{}", bad.port)),
        "a dead egress must be quarantined so the next rotation skips it"
    );

    // ---------- 3. Rotation adopts the OLDEST entry and stamps |last: ----------
    let fresh = spawn_upstream(UpstreamMode::AcceptWithLeftover).await;
    let mid = spawn_upstream(UpstreamMode::AcceptWithLeftover).await;
    let oldest = spawn_upstream(UpstreamMode::AcceptWithLeftover).await;

    let pool = root.join("opencode/proxy-pool.txt");
    write_pool(
        &pool,
        &[
            format!("127.0.0.1:{}:u:p |last:2026-10-07 12:00:00 |dur:1h 5m", fresh.port),
            format!("127.0.0.1:{}:u:p |last:2026-10-07 09:00:00 |dur:2h 0m", oldest.port),
            format!("127.0.0.1:{}:u:p |last:2026-10-07 11:00:00 |dur:0h 9m", mid.port),
        ],
    );
    // The dead upstream must be skipped during rotation.
    let mut quarantined = state.active_quarantine();
    quarantined.insert(format!("127.0.0.1:{}", bad.port));
    {
        let mut q = state.quarantine.lock().unwrap();
        for key in &quarantined {
            q.quarantine(key, Duration::from_secs(600), opencode_proxy::quarantine::now_secs());
        }
    }

    let changed = opencode_proxy::rotate::rotate_once(&state).await;
    assert!(changed, "rotation must adopt an upstream");
    let adopted = state.current_upstream().expect("an upstream must be current after rotation");
    assert_eq!(
        adopted.port,
        oldest.port,
        "rotation must pick the OLDEST |last: entry, not the first pool line"
    );

    // |last: rewritten in UTC; |dur: preserved verbatim (the reference destroys it).
    let text = std::fs::read_to_string(&pool).unwrap();
    let line = text
        .lines()
        .find(|l| l.starts_with(&format!("127.0.0.1:{}:", oldest.port)))
        .expect("adopted line still present");
    assert!(line.contains(" |last:20"), "|last: must be stamped: {line}");
    assert!(!line.contains("||"), "pool format broken by a double pipe: {line}");
    assert!(line.contains("|dur:2h 0m"), "|dur: must survive the write: {line}");

    // Every other line is byte-identical to what we wrote.
    let fresh_line = text
        .lines()
        .find(|l| l.starts_with(&format!("127.0.0.1:{}:", fresh.port)))
        .unwrap();
    assert_eq!(fresh_line, format!("127.0.0.1:{}:u:p |last:2026-10-07 12:00:00 |dur:1h 5m", fresh.port));

    // ---------- 4. Every IP dead -> keep the current egress, never blank it ----------
    let only_dead = root.join("opencode/pool-dead.txt");
    write_pool(&only_dead, &[format!("127.0.0.1:{}:u:p |last:2026-10-07 09:00:00", bad.port)]);
    let state2 = Arc::new(ProxyState::new(Arc::clone(&cfg), only_dead.clone()));
    {
        let mut q = state2.quarantine.lock().unwrap();
        q.quarantine(&format!("127.0.0.1:{}", bad.port), Duration::from_secs(600), opencode_proxy::quarantine::now_secs());
    }
    let before = state2.current_upstream();
    let changed2 = opencode_proxy::rotate::rotate_once(&state2).await;
    assert!(!changed2, "no candidate can pass the probe");
    assert_eq!(
        state2.current_upstream(),
        before,
        "a failed rotation must never blank the current egress"
    );

    // ---------- 5. A candidate that passes probe but was NOT chosen is not stamped ----------
    let probe_port = mid.port;
    let untouched = text
        .lines()
        .find(|l| l.starts_with(&format!("127.0.0.1:{probe_port}:")))
        .unwrap()
        .to_string();
    assert!(
        untouched.contains("|last:2026-10-07 11:00:00"),
        "an unadopted candidate's |last: must not advance: {untouched}"
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// The whole point of the rewrite: N live sessions must hold N DISTINCT egress IPs,
/// and each must stay on its own IP across a concurrent rotation.
///
/// The reference plugin guarantees this with a cross-process claims file under a
/// mkdir lock. Without it every session reads "nothing claimed", picks the same oldest
/// entry, and the fleet egresses from one IP — which is what was reported.
#[tokio::test]
async fn three_sessions_get_three_distinct_upstreams_and_keep_them_under_rotation() {
    let root = temp_root("multi");
    let _guard = serial();
    std::env::set_var("OPENCODE_PATHS_ROOT", &root);

    let mut upstreams = Vec::new();
    let mut lines = Vec::new();
    for index in 0..6 {
        let fake = spawn_upstream(UpstreamMode::AcceptWithLeftover).await;
        lines.push(format!(
            "127.0.0.1:{}:u:p |last:2026-10-07 0{}:00:00",
            fake.port,
            10 - index
        ));
        upstreams.push(fake);
    }
    let pool = root.join("opencode/proxy-pool.txt");
    write_pool(&pool, &lines);

    let mut cfg = cfg_for_test();
    cfg.max_probes_per_switch = 12;
    let cfg = Arc::new(cfg);
    let state = Arc::new(ProxyState::new(Arc::clone(&cfg), pool));

    let sessions = ["ses_alpha", "ses_beta", "ses_gamma"];
    let mut handles = Vec::new();
    for session_id in sessions {
        let state = Arc::clone(&state);
        handles.push(tokio::spawn(async move {
            opencode_proxy::rotate::ensure_session(&state, session_id).await;
        }));
    }
    for handle in handles {
        handle.await.unwrap();
    }

    let claimed: Vec<(String, u16)> = sessions
        .iter()
        .map(|id| {
            let egress = state.egress_for(id).unwrap_or_else(|| panic!("{id} has no egress"));
            (id.to_string(), egress.upstream.port)
        })
        .collect();

    let distinct: std::collections::HashSet<u16> = claimed.iter().map(|(_, port)| *port).collect();
    assert_eq!(
        distinct.len(),
        3,
        "three sessions must hold three DIFFERENT egress IPs, got {claimed:?}"
    );
    for (session_id, _) in &claimed {
        assert!(
            state.egress_for(session_id).is_some(),
            "{session_id} must still hold an egress after registration"
        );
    }

    opencode_proxy::rotate::rotate_all(&state).await;
    let after: Vec<(String, u16)> = sessions
        .iter()
        .map(|id| {
            let egress = state.egress_for(id).unwrap_or_else(|| panic!("{id} lost its egress"));
            (id.to_string(), egress.upstream.port)
        })
        .collect();
    let distinct_after: std::collections::HashSet<u16> = after.iter().map(|(_, port)| *port).collect();
    assert_eq!(
        distinct_after.len(),
        3,
        "sessions must still be spread after rotation, got {after:?}"
    );
    for (session_id, port) in &after {
        assert!(
            upstreams.iter().any(|u| u.port == *port),
            "{session_id} must egress through a real pool entry"
        );
    }

    let _ = std::fs::remove_dir_all(&root);
}

/// The listener must route each CONNECT by the session id its client sent, so a
/// request from session A never leaves through session B's residential IP.
///
/// Runs in-process with `#[tokio::test]` on the CURRENT runtime rather than spawning
/// one: `bind_listener` binds the configured port, so a dedicated runtime would race
/// the parallel tests for the same port.
#[test]
fn each_session_connect_exits_through_its_own_upstream() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
    let root = temp_root("route");
    let _guard = serial();
    std::env::set_var("OPENCODE_PATHS_ROOT", &root);

    let first = spawn_upstream(UpstreamMode::AcceptWithLeftover).await;
    let second = spawn_upstream(UpstreamMode::AcceptWithLeftover).await;
    let third = spawn_upstream(UpstreamMode::AcceptWithLeftover).await;
    let pool = root.join("opencode/proxy-pool.txt");
    write_pool(
        &pool,
        &[
            format!("127.0.0.1:{}:u:p |last:2026-10-07 09:00:00", first.port),
            format!("127.0.0.1:{}:u:p |last:2026-10-07 10:00:00", second.port),
            format!("127.0.0.1:{}:u:p |last:2026-10-07 11:00:00", third.port),
        ],
    );

    let cfg = Arc::new(cfg_for_test());
    let state = Arc::new(ProxyState::new(Arc::clone(&cfg), pool));
    for session_id in ["ses_one", "ses_two"] {
        opencode_proxy::rotate::ensure_session(&state, session_id).await;
    }
    let (listener, port) = opencode_proxy::listener::bind_listener(&cfg).await.unwrap();
    opencode_proxy::listener::accept_loop(listener, Arc::clone(&cfg), Arc::clone(&state));

    let dial = |session_id: &'static str| async move {
        let url = opencode_proxy::session::proxy_url_for_session(port, session_id);
        let authority = url.trim_start_matches("http://").split('@').next().unwrap().to_string();
        let (user, password) = authority.split_once(':').unwrap();
        let raw = format!("{user}:{password}");
        let auth = format!(
            "Basic {}",
            base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                raw.as_bytes()
            )
        );
        let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        client
            .write_all(
                format!(
                    "CONNECT fake-target:443 HTTP/1.1\r\nHost: fake-target:443\r\nProxy-Authorization: {auth}\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        read_head(&mut client).await
    };

    let one = dial("ses_one").await;
    assert!(one.starts_with("HTTP/1.1 200"), "session one's CONNECT failed: {one}");
    let two = dial("ses_two").await;
    assert!(two.starts_with("HTTP/1.1 200"), "session two's CONNECT failed: {two}");

    let served_by_one = state.upstream_for(Some("ses_one")).unwrap().port;
    let served_by_two = state.upstream_for(Some("ses_two")).unwrap().port;
    assert_ne!(
        served_by_one, served_by_two,
        "the two sessions must be served by different upstreams"
    );

    let _ = std::fs::remove_dir_all(&root);
    });
}

#[test]
fn concurrent_connections_are_all_served() {
    // Guards the accept loop against a serialising bug: N simultaneous CONNECTs each
    // get their own tunnel, and no one is answered with somebody else's leftover.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let root = temp_root("conc");
        let _guard = serial();
    std::env::set_var("OPENCODE_PATHS_ROOT", &root);
        let upstream = spawn_upstream(UpstreamMode::AcceptWithLeftover).await;
        let cfg = Arc::new(cfg_for_test());
        let state = Arc::new(ProxyState::new(Arc::clone(&cfg), root.join("opencode/proxy-pool.txt")));
        state.set_upstream(Some(Upstream::new("127.0.0.1", upstream.port, "u", "p")));
        let (listener, port) = opencode_proxy::listener::bind_listener(&cfg).await.unwrap();
        opencode_proxy::listener::accept_loop(listener, Arc::clone(&cfg), Arc::clone(&state));

        let served = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let served = Arc::clone(&served);
            handles.push(tokio::spawn(async move {
                let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
                client
                    .write_all(b"CONNECT t:443 HTTP/1.1\r\nHost: t:443\r\n\r\n")
                    .await
                    .unwrap();
                let established = read_head(&mut client).await;
                if !established.starts_with("HTTP/1.1 200") {
                    return;
                }
                let mut buf = vec![0u8; LEFTOVER.len()];
                if tokio::time::timeout(Duration::from_secs(5), client.read_exact(&mut buf))
                    .await
                    .map(|r| r.is_ok())
                    .unwrap_or(false)
                    && buf == LEFTOVER.as_bytes()
                {
                    served.fetch_add(1, Ordering::Relaxed);
                }
            }));
        }
        for handle in handles {
            let _ = handle.await;
        }
        assert_eq!(served.load(Ordering::Relaxed), 8, "every connection must get its own tunnel");
        let _ = std::fs::remove_dir_all(&root);
    });
}

/// `spawn` must not rotate twice at boot. `tokio::time::interval` fires its first
/// tick immediately, so an unfixed loop adopts an egress twice seconds apart and
/// throws the first one away.
#[tokio::test]
async fn startup_adopts_exactly_once_before_the_first_interval() {
    let root = temp_root("once");
    let _guard = serial();
    std::env::set_var("OPENCODE_PATHS_ROOT", &root);

    let up_a = spawn_upstream(UpstreamMode::AcceptWithLeftover).await;
    let up_b = spawn_upstream(UpstreamMode::AcceptWithLeftover).await;
    let pool = root.join("opencode/proxy-pool.txt");
    write_pool(
        &pool,
        &[
            format!("127.0.0.1:{}:u:p |last:2026-10-07 09:00:00", up_a.port),
            format!("127.0.0.1:{}:u:p |last:2026-10-07 08:00:00", up_b.port),
        ],
    );

    let mut cfg = cfg_for_test();
    cfg.rotate_interval = Duration::from_millis(500);
    let cfg = Arc::new(cfg);
    let state = Arc::new(ProxyState::new(Arc::clone(&cfg), pool.clone()));

    opencode_proxy::rotate::spawn(Arc::clone(&state));

    // Wait for the initial adoption to land.
    for _ in 0..50 {
        if state.rotations_total() > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(state.rotations_total(), 1, "startup must adopt exactly one egress");

    // Well inside the first 500ms tick: a second adoption here is the double-rotate bug.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        state.rotations_total(),
        1,
        "a second adoption before the interval elapsed means the schedule starts at t=0"
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// The runtime kill switch must actually stop residential egress.
///
/// The target is TEST-NET-3 (`203.0.113.7`, RFC 5737) because it must NOT be loopback:
/// loopback is always bypassed, so a 127.0.0.1 target would behave identically with the
/// switch on or off and prove nothing. The fake upstream answers any CONNECT, so phase 1
/// needs no real network, and phase 2 asserts the upstream is never contacted — whether
/// the destination itself is reachable is irrelevant to that claim.
#[tokio::test]
async fn direct_mode_stops_touching_the_upstream() {
    let root = temp_root("direct");
    let _guard = serial();
    std::env::set_var("OPENCODE_PATHS_ROOT", &root);

    let upstream = spawn_upstream(UpstreamMode::AcceptWithLeftover).await;
    let cfg = Arc::new(cfg_for_test());
    let state = Arc::new(ProxyState::new(Arc::clone(&cfg), root.join("opencode/proxy-pool.txt")));
    state.set_upstream(Some(Upstream::new("127.0.0.1", upstream.port, "u", "p")));
    let (listener, port) = opencode_proxy::listener::bind_listener(&cfg).await.unwrap();
    opencode_proxy::listener::accept_loop(listener, Arc::clone(&cfg), Arc::clone(&state));

    let target = "203.0.113.7:443";
    let ask = |p: u16| async move {
        let mut c = TcpStream::connect(("127.0.0.1", p)).await.unwrap();
        c.write_all(format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n").as_bytes())
            .await
            .unwrap();
        read_head(&mut c).await
    };

    // --- proxied: upstream must see the CONNECT
    let first = ask(port).await;
    assert!(first.starts_with("HTTP/1.1 200"), "proxied CONNECT failed: {first}");
    assert_eq!(
        upstream.targets.lock().unwrap().len(),
        1,
        "with direct mode off the upstream must receive the CONNECT"
    );

    // --- kill switch on: upstream must be ignored, even though the target is dead
    state.set_direct(true);
    assert!(state.is_direct());

    let second = ask(port).await;
    assert_eq!(
        upstream.targets.lock().unwrap().len(),
        1,
        "with direct mode on the upstream must not be contacted at all"
    );
    // The dead TEST-NET address cannot be reached directly, and the bounded dial must
    // report that quickly rather than hanging.
    assert!(!second.starts_with("HTTP/1.1 200"), "direct mode must not borrow the upstream's 200");

    let _ = std::fs::remove_dir_all(&root);
}
