//! Diagnostic: prints what the audio engine sees (devices, apps, meters) without
//! changing anything. Useful for troubleshooting and bug reports.
//!
//!     cargo run --example audio_probe -- [seconds-to-watch]

use glass_mixer::audio::{AudioHandle, ServiceStatus, to_percent};
use std::time::{Duration, Instant};

fn main() {
    let watch_secs: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let audio = AudioHandle::start();

    let start = Instant::now();
    while audio.snapshot().status == ServiceStatus::Starting && start.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(20));
    }
    let snap = audio.snapshot();
    println!("status: {:?} (ready after {:?})", snap.status, start.elapsed());
    println!("default switching available: {}", snap.can_switch_default);
    for (label, ep) in [("output", &snap.output), ("input", &snap.input)] {
        match ep {
            Some(e) => println!(
                "{label}: {} — {}%{} balance={:?}",
                e.name,
                to_percent(e.volume),
                if e.muted { " (muted)" } else { "" },
                e.balance
            ),
            None => println!("{label}: none"),
        }
    }
    println!("outputs:");
    for d in &snap.outputs {
        println!("  {} {}", if d.is_default { "*" } else { " " }, d.name);
    }
    println!("inputs:");
    for d in &snap.inputs {
        println!("  {} {}", if d.is_default { "*" } else { " " }, d.name);
    }
    println!("apps ({}):", snap.apps.len());
    for a in &snap.apps {
        println!(
            "  {:<28} {:>3}%{} {:?} sessions={} key={:?} balance={:?}",
            a.name,
            to_percent(a.volume),
            if a.muted { " M" } else { "  " },
            a.activity,
            a.session_count,
            a.key,
            a.balance
        );
    }

    if watch_secs > 0 {
        audio.set_meters_wanted(true);
        let (tx, rx) = crossbeam_channel::unbounded();
        audio.add_listener(move || {
            let _ = tx.send(());
        });
        let end = Instant::now() + Duration::from_secs(watch_secs);
        let mut last = audio.snapshot();
        println!("watching for {watch_secs}s (change volumes in Windows to see updates)...");
        while Instant::now() < end {
            if rx.recv_timeout(Duration::from_millis(200)).is_ok() {
                let snap = audio.snapshot();
                if snap != last {
                    if let Some(o) = &snap.output {
                        println!(
                            "[{:>6.2}s] output {}%{} | apps: {}",
                            start.elapsed().as_secs_f32(),
                            to_percent(o.volume),
                            if o.muted { " muted" } else { "" },
                            snap.apps
                                .iter()
                                .map(|a| format!(
                                    "{} {}%{}",
                                    a.name,
                                    to_percent(a.volume),
                                    if a.muted { "M" } else { "" }
                                ))
                                .collect::<Vec<_>>()
                                .join(", ")
                        );
                    }
                    last = snap;
                }
            }
        }
        let m = audio.meters();
        println!("last meters: output={:.2} input={:.2} apps={:?}", m.output, m.input, m.apps);
    }

    for n in audio.take_notices() {
        println!("notice: {:?} {}", n.level, n.text);
    }
    audio.shutdown();
}
