//! Measure what the hotkey work rests on, instead of reading about it.
//!
//! Everything this prints is a fact the conflict-reporting code needs and that
//! no documentation settles: whether the exclusive registration mode arbitrates
//! at all, whether it arbitrates *across processes* or only inside one, and
//! what the system's own shortcut table actually looks like on this OS. Run it,
//! paste the output into `docs/TASK-hotkey-native.md`, and only then write code
//! against the answer.
//!
//! ```sh
//! cargo run --example hotkey_probe
//! ```
//!
//! Every registration it makes is released before the next line prints, so it
//! never leaves a combination taken. It deliberately probes `⌃⌥⇧⌘F19` — four
//! modifiers on a key most keyboards do not have — so that even a leak would
//! take nothing a person could press.

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("hotkey_probe measures macOS; nothing on Windows has been measured yet");
}

#[cfg(target_os = "macos")]
fn main() {
    use nudge_web::hotkey;
    use nudge_web::platform::{hotkey_carbon, keycode, symbolic};

    let mode = std::env::args().nth(1).unwrap_or_default();
    let probe = hotkey::parse("Ctrl+Alt+Shift+Cmd+F19").expect("the probe combination parses");

    // ---- the child halves of the cross-process questions -------------------
    if mode == "--hold-shared" {
        // The shape the real app has: the main instance binds NON-exclusively.
        match hotkey_carbon::register(&probe) {
            Ok(held) => {
                println!(
                    "child: holding {} non-exclusively",
                    hotkey::format_native(&probe)
                );
                std::thread::sleep(std::time::Duration::from_secs(3));
                drop(held);
            }
            Err(e) => println!("child: could not hold it: {e}"),
        }
        return;
    }
    if mode == "--hold-exclusive" {
        match hotkey_carbon::register_exclusive(&probe) {
            Ok(held) => {
                println!(
                    "child: holding {} exclusively",
                    hotkey::format_native(&probe)
                );
                // Long enough for the parent to try, short enough that a
                // forgotten child releases it on its own.
                std::thread::sleep(std::time::Duration::from_secs(3));
                drop(held);
            }
            Err(e) => println!("child: could not hold it: {e}"),
        }
        return;
    }

    println!("== registration ==");
    let non_exclusive = hotkey_carbon::register(&probe);
    println!(
        "non-exclusive registration of {}: {}",
        hotkey::format_native(&probe),
        match &non_exclusive {
            Ok(_) => "ok".to_string(),
            Err(e) => e.to_string(),
        }
    );

    // Q1 — does exclusive collide with our own non-exclusive binding, in the
    // same process? If it does, `assess` can never be called while we hold the
    // combination without reporting us to ourselves.
    println!(
        "exclusive probe while WE hold it non-exclusively (same process): {:?}",
        hotkey_carbon::assess(&probe)
    );
    drop(non_exclusive);
    println!(
        "exclusive probe with nothing held: {:?}",
        hotkey_carbon::assess(&probe)
    );

    // Q2 — does exclusive arbitrate across processes? If it does not, source 2
    // of the conflict report is worthless and must be said to be, rather than
    // left as a check that never fires.
    println!("\n== across processes ==");
    let exe = std::env::current_exe().expect("own path");
    match std::process::Command::new(exe)
        .arg("--hold-exclusive")
        .spawn()
    {
        Ok(mut child) => {
            std::thread::sleep(std::time::Duration::from_millis(700));
            println!(
                "exclusive probe while a CHILD holds it exclusively: {:?}",
                hotkey_carbon::assess(&probe)
            );
            println!(
                "  and a NON-exclusive registration against that exclusive holder: {}",
                match hotkey_carbon::register(&probe) {
                    Ok(_) =>
                        "ok — the app can still bind what somebody holds exclusively".to_string(),
                    Err(e) => e.to_string(),
                }
            );
            let _ = child.wait();
            println!(
                "exclusive probe after the child exited: {:?}",
                hotkey_carbon::assess(&probe)
            );
        }
        Err(e) => println!("could not spawn the child probe: {e}"),
    }

    // Q2b — the scenario the settings window is actually in: the running Nudge
    // holds the combination NON-exclusively, in another process. If the probe
    // sees that, `HeldByNudge` is a case that really happens and the settings
    // process must be able to recognise its own app; if it does not, the
    // running instance is invisible to the check and that has to be said.
    let exe = std::env::current_exe().expect("own path");
    match std::process::Command::new(exe).arg("--hold-shared").spawn() {
        Ok(mut child) => {
            std::thread::sleep(std::time::Duration::from_millis(700));
            println!(
                "exclusive probe while a CHILD holds it NON-exclusively: {:?}",
                hotkey_carbon::assess(&probe)
            );
            let _ = child.wait();
        }
        Err(e) => println!("could not spawn the shared-holder child: {e}"),
    }

    // Q3 — the system's own table: is it readable, what shape, and are
    // disabled entries included?
    println!("\n== CopySymbolicHotKeys ==");
    let table = symbolic::read_symbolic_hotkeys();
    println!("{} entries", table.len());
    println!(
        "{} enabled, {} disabled — a disabled row present at all is what lets \
         a turned-off system shortcut read as free",
        table.iter().filter(|e| e.enabled).count(),
        table.iter().filter(|e| !e.enabled).count()
    );
    for probe_label in ["Cmd+Space", "Ctrl+Left", "Cmd+Shift+3", "Cmd+Shift+5"] {
        let hk = hotkey::parse(probe_label).expect("parses");
        let Some(c) = keycode::keycode_for(hk.key.as_str()) else {
            continue;
        };
        println!(
            "  {probe_label:14} → {:?}",
            symbolic::held_by_system(&table, c, keycode::carbon_modifiers(hk.modifiers))
        );
    }
}
