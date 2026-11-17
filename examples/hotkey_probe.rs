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

    if mode == "--hold-shared" {
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

    println!(
        "exclusive probe while WE hold it non-exclusively (same process): {:?}",
        hotkey_carbon::assess(&probe)
    );
    drop(non_exclusive);
    println!(
        "exclusive probe with nothing held: {:?}",
        hotkey_carbon::assess(&probe)
    );

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
