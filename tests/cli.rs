//! The `win_ter` binary, run as a user runs it: against stand-in Protons (a script that
//! reports what it sees) and stand-in homes, each a scratch folder of its own. The tests that start
//! a real sandbox skip, saying so, where bubblewrap can't make one (no bwrap, or no user
//! namespaces, as in some containers).

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// A scratch folder for one test, standing in for a home: empty, gone afterwards.
fn scratch_home(tag: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("win_ter_cli_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    home
}

/// Whether bubblewrap can make a sandbox here; where not, the test says it skips.
fn sandboxes() -> bool {
    let made =
        Command::new("bwrap").args(["--ro-bind", "/", "/", "true"]).stderr(Stdio::null()).status();
    let can = matches!(made, Ok(status) if status.success());
    if !can {
        eprintln!("skipped: bubblewrap can't make a sandbox here");
    }
    can
}

/// A stand-in Proton in `dir`: its script (`body`), its version file, and — where `wayland` says
/// so — Wine's Wayland driver and the switch for it, which is what makes a Proton run games on
/// Wayland.
fn fake_proton(dir: &Path, version: &str, wayland: bool, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir).unwrap();
    let switch = if wayland { "# reads PROTON_ENABLE_WAYLAND\n" } else { "" };
    std::fs::write(dir.join("proton"), format!("{body}{switch}")).unwrap();
    std::fs::set_permissions(dir.join("proton"), std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(dir.join("version"), format!("{version}\n")).unwrap();
    if wayland {
        let driver = dir.join("files/lib/wine/x86_64-unix/winewayland.so");
        std::fs::create_dir_all(driver.parent().unwrap()).unwrap();
        std::fs::write(driver, "").unwrap();
    }
}

/// A `shortcuts.vdf` the way Steam writes one (binary KeyValues): a map per non-Steam game an
/// account added, with the id Steam gave it, its title, and its .exe, quoted.
fn shortcuts_vdf(games: &[(u32, &str, &Path)]) -> Vec<u8> {
    let mut vdf = b"\x00shortcuts\x00".to_vec();
    for (index, (id, title, exe)) in games.iter().enumerate() {
        vdf.extend(format!("\x00{index}\x00\x02appid\x00").bytes());
        vdf.extend(id.to_le_bytes());
        vdf.extend(
            format!("\x01AppName\x00{title}\x00\x01Exe\x00\"{}\"\x00\x08", exe.display()).bytes(),
        );
    }
    vdf.extend(b"\x08\x08");
    vdf
}

/// A home whose Steam has a non-Steam game added — its .exe, and the prefix Proton made under the
/// id Steam gave it, holding a marker file — for `--prefix-continue` and `--take-ownership`: the
/// home, the .exe, the prefix's folder, and where a stand-in Proton goes.
fn home_with_steam_prefix(tag: &str) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
    let home = scratch_home(tag);
    let steam = home.join(".local/share/Steam");
    let (exe, prefix) =
        (home.join("games/ice/Game.exe"), steam.join("steamapps/compatdata/3640112877"));
    for dir in
        [&home.join("games/ice"), &prefix.join("pfx"), &steam.join("userdata/76197245/config")]
    {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(&exe, "MZ").unwrap();
    std::fs::write(prefix.join("pfx/marker"), "Steam's").unwrap();
    let shortcuts = shortcuts_vdf(&[(3_640_112_877, "itchio-Cirno-ice-shave", &exe)]);
    std::fs::write(steam.join("userdata/76197245/config/shortcuts.vdf"), shortcuts).unwrap();
    let proton = home.join("protons/stand-in");
    (home, exe, prefix, proton)
}

/// The game runs through Proton inside a sandbox, tried here with a stand-in Proton: a script that
/// reports what it sees there. Proton's verb, then the game and its arguments as given; a home of
/// its own, in the game's own folder under `~/Games/manual_run_proton_prefixes`, writable; no
/// network but the loopback; nothing of the user's home but the game's folders; the Wayland socket,
/// the driver's switch and no X display; the syscall filter in place, and no user namespace to be
/// made. The game's exit status is the run's.
#[test]
fn the_game_runs_in_a_sandbox_that_sees_only_its_own_folders() {
    if !sandboxes() {
        return;
    }
    let home = scratch_home("sandbox");
    let (game, proton, runtime) =
        (home.join("games/Some Game"), home.join("protons/stand-in"), home.join("run"));
    for dir in [&game, &runtime] {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(home.join("secret.txt"), "the user's own").unwrap();
    std::fs::write(game.join("game.exe"), "MZ").unwrap();
    std::fs::write(runtime.join("wayland-test"), "").unwrap(); // stands in for the compositor's socket
    let report = game.join("report");
    let script = format!(
        r#"#!/bin/sh
{{
echo "verb=$1"; shift
echo "game=$1"; shift
echo "args=$*"
echo "home=$HOME"
echo "wayland=$WAYLAND_DISPLAY $PROTON_ENABLE_WAYLAND"
echo "display=${{DISPLAY:-none}}"
echo "data=$STEAM_COMPAT_DATA_PATH"
echo "interfaces=$(grep -c : /proc/net/dev)"
[ -e '{secret}' ] && echo secret=seen || echo secret=hidden
[ -e "$XDG_RUNTIME_DIR/wayland-test" ] && echo socket=there || echo socket=missing
touch "$HOME/written" && echo home=writable
grep '^Seccomp:' /proc/self/status
unshare -U true 2>/dev/null && echo userns=made || echo userns=refused
}} > '{report}'
exit 7
"#,
        secret = home.join("secret.txt").display(),
        report = report.display(),
    );
    fake_proton(&proton, "100 stand-in-proton", true, &script);

    let out = Command::new(env!("CARGO_BIN_EXE_win_ter"))
        .args(["--no-performance", "--proton"])
        .arg(&proton)
        .arg(game.join("game.exe"))
        .args(["-windowed", "two words"])
        .env("HOME", &home)
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("WAYLAND_DISPLAY", "wayland-test")
        .env_remove("DISPLAY")
        .env_remove("XDG_CONFIG_HOME")
        .output()
        .expect("run win_ter");
    let said = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(7), "the game's status is the run's: {said}");
    assert!(said.contains("win_ter: stand-in-proton on Wayland, network off"), "{said}");

    let prefixes: Vec<PathBuf> = std::fs::read_dir(home.join("Games/manual_run_proton_prefixes"))
        .expect("the game's own folder is made under ~/Games/manual_run_proton_prefixes")
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(prefixes.len(), 1, "{prefixes:?}");
    let prefix = &prefixes[0];
    assert!(prefix.file_name().unwrap().to_string_lossy().starts_with("Some Game-"), "{prefix:?}");
    let report = std::fs::read_to_string(&report).expect("the stand-in reported");
    let expected = [
        "verb=waitforexitandrun".to_string(),
        format!("game={}", game.join("game.exe").display()),
        "args=-windowed two words".to_string(),
        format!("home={}", prefix.join("home").display()),
        "wayland=wayland-test 1".to_string(),
        "display=none".to_string(),
        format!("data={}", prefix.display()),
        "interfaces=1".to_string(),
        "secret=hidden".to_string(),
        "socket=there".to_string(),
        "home=writable".to_string(),
        // The syscall filter (x86-64's) holds the game: no namespace of its own, by it or by bubblewrap.
        format!("Seccomp:\t{}", if cfg!(target_arch = "x86_64") { 2 } else { 0 }),
        "userns=refused".to_string(),
    ];
    assert_eq!(report.lines().collect::<Vec<_>>(), expected);
    assert!(
        prefix.join("home/written").exists(),
        "what the game writes in its home stays, in its own folder"
    );
    let _ = std::fs::remove_dir_all(&home);
}

/// `--dry-run` prints the sandbox's command and runs nothing, the .exe and a Proton named from
/// another folder in it whole; what isn't a game, or isn't in the folder given as the game's, is
/// refused before anything runs.
#[test]
fn a_dry_run_says_what_would_run_and_what_cant_is_refused() {
    let home = scratch_home("dry_run");
    let (game, proton) = (home.join("game"), home.join("proton"));
    std::fs::create_dir_all(&game).unwrap();
    std::fs::write(game.join("game.exe"), "MZ").unwrap();
    fake_proton(&proton, "100 stand-in-proton", false, "#!/bin/sh\nexit 0\n");
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_win_ter"))
            .args(args)
            .env("HOME", &home)
            .env_remove("WAYLAND_DISPLAY")
            .output()
            .unwrap()
    };
    let (exe, proton) = (game.join("game.exe"), proton.to_str().unwrap().to_string());
    let out =
        run(&["--dry-run", "--no-performance", "--proton", &proton, exe.to_str().unwrap(), "-arg"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let printed = String::from_utf8_lossy(&out.stdout);
    assert!(printed.starts_with("bwrap --die-with-parent --new-session"), "{printed}");
    assert!(printed.contains("--unshare-net") && printed.trim_end().ends_with("-arg"), "{printed}");
    assert!(!home.join("Games").exists(), "a dry run makes nothing");
    let relative = Command::new(env!("CARGO_BIN_EXE_win_ter"))
        .args(["--dry-run", "--no-performance", "--proton", "proton", "game/game.exe"])
        .current_dir(&home)
        .env("HOME", &home)
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .unwrap();
    let whole = |path: &Path| std::fs::canonicalize(path).unwrap().display().to_string();
    let ran = format!(" sh {} {}", whole(Path::new(&proton)), whole(&exe));
    assert!(
        String::from_utf8_lossy(&relative.stdout).contains(&ran),
        "paths named from a folder run whole: {relative:?}"
    );
    let by_folder =
        run(&["--dry-run", "--no-performance", "--proton", &proton, game.to_str().unwrap()]);
    assert!(by_folder.status.success(), "{}", String::from_utf8_lossy(&by_folder.stderr));
    assert!(
        String::from_utf8_lossy(&by_folder.stdout).trim_end().ends_with("/game/game.exe"),
        "a folder with one .exe runs it: {by_folder:?}"
    );

    let missing = run(&["--proton", &proton, "/no/such/game.exe"]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&missing.stderr).contains("win_ter: /no/such/game.exe is no file"),
        "{missing:?}"
    );
    let elsewhere = run(&[
        "--proton",
        &proton,
        "--game-dir",
        home.join("proton").to_str().unwrap(),
        exe.to_str().unwrap(),
    ]);
    assert_eq!(elsewhere.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&elsewhere.stderr).contains("isn't inside the game's folder"),
        "{elsewhere:?}"
    );
    let unknown = run(&["--proton", "no-such-proton-anywhere", exe.to_str().unwrap()]);
    assert_eq!(unknown.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&unknown.stderr).contains("no Proton here is called that"),
        "{unknown:?}"
    );
    let _ = std::fs::remove_dir_all(&home);
}

/// No folder the game may write to may be, or hold, the user's own folders: an .exe right in the
/// home, or right in Downloads, is refused, as are `--allow` and `--prefix` of such a folder; a
/// game in a folder of its own inside Downloads is fine. Nothing runs either way (dry runs).
#[test]
fn folders_too_broad_for_the_game_to_write_to_are_refused() {
    let home = scratch_home("broad");
    let proton = home.join("protons/stand-in");
    fake_proton(&proton, "100 stand-in-proton", false, "#!/bin/sh\nexit 0\n");
    for exe in ["loose.exe", "Downloads/setup.exe", "Downloads/Some Game/game.exe"] {
        std::fs::create_dir_all(home.join(exe).parent().unwrap()).unwrap();
        std::fs::write(home.join(exe), "MZ").unwrap();
    }
    let run = |args: &[&str]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_win_ter"));
        command
            .args(["--dry-run", "--no-performance", "--proton"])
            .arg(&proton)
            .args(args)
            .env("HOME", &home);
        for key in [
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_STATE_HOME",
            "XDG_CACHE_HOME",
            "WAYLAND_DISPLAY",
        ] {
            command.env_remove(key);
        }
        command.output().unwrap()
    };
    let path = |relative: &str| home.join(relative).to_str().unwrap().to_string();
    let refused = |args: &[&str], said: &str| {
        let out = run(args);
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains(said), "{args:?}: {stderr}");
    };
    refused(
        &[&path("loose.exe")],
        &format!(
            "the game could write to all of its folder, {}, and it is your home folder",
            home.display()
        ),
    );
    refused(
        &[&path("Downloads/setup.exe")],
        "and it is ~/Downloads, one of your own folders (put the .exe in a folder of its own",
    );
    let game = path("Downloads/Some Game/game.exe");
    refused(&["--allow", home.to_str().unwrap(), &game], "--allow");
    refused(&["--prefix", &path("Documents"), &game], "it is ~/Documents, one of your own folders");
    let fine = run(&[&game]);
    assert!(fine.status.success(), "{}", String::from_utf8_lossy(&fine.stderr));
    let _ = std::fs::remove_dir_all(&home);
}

/// `--prefix-continue` runs the game on the prefix Steam made for its .exe, where it is, and
/// `--take-ownership` on a copy of it in the game's own folder — which a dry run only says it would
/// make. Refused: taking over where the game's own folder is already there; either, where no
/// launcher has a prefix for the .exe, or where several have and their menu can't show (stderr is
/// no terminal here), every one named; and either beside --prefix, or both together.
#[test]
fn a_run_goes_on_from_the_prefix_steam_made_or_says_why_not() {
    let (home, exe, steams, proton) = home_with_steam_prefix("past_prefix");
    fake_proton(&proton, "100 stand-in-proton", false, "#!/bin/sh\nexit 0\n");
    let run = |exe: &Path, args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_win_ter"))
            .args(["--dry-run", "--no-performance", "--proton"])
            .arg(&proton)
            .args(args)
            .arg(exe)
            .env("HOME", &home)
            .env_remove("WAYLAND_DISPLAY")
            .output()
            .unwrap()
    };
    let printed = |out: &Output| String::from_utf8_lossy(&out.stdout).into_owned();
    let said = |out: &Output| String::from_utf8_lossy(&out.stderr).into_owned();

    let continued = run(&exe, &["--prefix-continue"]);
    assert!(continued.status.success(), "{}", said(&continued));
    assert!(
        printed(&continued).contains(&format!("STEAM_COMPAT_DATA_PATH {} ", steams.display())),
        "{}",
        printed(&continued)
    );

    let owned = home.join("Games/manual_run_proton_prefixes");
    let taken = run(&exe, &["--take-ownership"]);
    assert!(taken.status.success(), "{}", said(&taken));
    assert!(
        said(&taken).contains(&format!(
            "would copy Steam's prefix for itchio-Cirno-ice-shave ({}) into {}/ice-",
            steams.display(),
            owned.display()
        )),
        "{}",
        said(&taken)
    );
    assert!(
        printed(&taken).contains(&format!("STEAM_COMPAT_DATA_PATH {}/ice-", owned.display())),
        "{}",
        printed(&taken)
    );
    assert!(!home.join("Games").exists(), "a dry run copies nothing");

    let mine = owned.join("mine");
    std::fs::create_dir_all(&mine).unwrap();
    let there = run(&exe, &["--take-ownership", "--prefix", mine.to_str().unwrap()]);
    assert_eq!(there.status.code(), Some(1));
    assert!(
        said(&there).contains(&format!(
            "{} is already there: the game has a prefix of its own",
            mine.display()
        )),
        "{}",
        said(&there)
    );

    std::fs::write(home.join("games/ice/Other.exe"), "MZ").unwrap();
    for flag in ["--prefix-continue", "--take-ownership"] {
        let none = run(&home.join("games/ice/Other.exe"), &[flag]);
        assert_eq!(none.status.code(), Some(1), "{flag}");
        assert!(
            said(&none).contains("no launcher here has a prefix for")
                && said(&none).contains("(looked in: Steam)"),
            "{flag}: {}",
            said(&none)
        );
    }

    let again = steams.with_file_name("3000000001");
    std::fs::create_dir_all(again.join("pfx")).unwrap();
    let both = shortcuts_vdf(&[
        (3_640_112_877, "itchio-Cirno-ice-shave", &exe),
        (3_000_000_001, "Ice, again", &exe),
    ]);
    std::fs::write(home.join(".local/share/Steam/userdata/76197245/config/shortcuts.vdf"), both)
        .unwrap();
    for flag in ["--prefix-continue", "--take-ownership"] {
        let several = run(&exe, &[flag]);
        assert_eq!(several.status.code(), Some(1), "{flag}");
        let said = said(&several);
        assert!(
            said.contains("more than one prefix was made for")
                && said.contains("the menu to pick one can't show"),
            "{flag}: {said}"
        );
        assert!(
            said.contains(&format!(
                "Steam's prefix for itchio-Cirno-ice-shave ({})",
                steams.display()
            )),
            "{flag}: {said}"
        );
        assert!(
            said.contains(&format!("Steam's prefix for Ice, again ({})", again.display())),
            "{flag}: {said}"
        );
    }

    for clash in [
        &["--prefix-continue", "--prefix", "/somewhere"][..],
        &["--prefix-continue", "--take-ownership"],
    ] {
        assert_eq!(run(&exe, clash).status.code(), Some(2), "{clash:?}");
    }
    let _ = std::fs::remove_dir_all(&home);
}

/// `--take-ownership` for real: the prefix Steam made is copied into the game's own folder, its
/// source left as it was, and the game runs on the copy.
#[test]
fn taking_over_the_prefix_steam_made_runs_the_game_on_a_copy() {
    if !sandboxes() {
        return;
    }
    let (home, exe, steams, proton) = home_with_steam_prefix("take_ownership");
    let report = home.join("games/ice/report");
    let script = format!(
        "#!/bin/sh\n{{ echo \"data=$STEAM_COMPAT_DATA_PATH\"; cat \"$STEAM_COMPAT_DATA_PATH/pfx/marker\"; }} > '{}'\n",
        report.display()
    );
    fake_proton(&proton, "100 stand-in-proton", false, &script);
    let out = Command::new(env!("CARGO_BIN_EXE_win_ter"))
        .args(["--no-performance", "--take-ownership", "--proton"])
        .arg(&proton)
        .arg(&exe)
        .env("HOME", &home)
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("DISPLAY")
        .output()
        .expect("run win_ter");
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{said}");
    assert!(said.contains("win_ter: copying Steam's prefix for itchio-Cirno-ice-shave"), "{said}");
    let owned: Vec<PathBuf> = std::fs::read_dir(home.join("Games/manual_run_proton_prefixes"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(owned.len(), 1, "only the game's own folder, nothing left beside it: {owned:?}");
    assert_eq!(
        std::fs::read_to_string(&report).unwrap(),
        format!("data={}\nSteam's", owned[0].display()),
        "the game ran on the copy"
    );
    assert_eq!(std::fs::read_to_string(owned[0].join("pfx/marker")).unwrap(), "Steam's");
    assert_eq!(
        std::fs::read_to_string(steams.join("pfx/marker")).unwrap(),
        "Steam's",
        "the source stays"
    );
    let _ = std::fs::remove_dir_all(&home);
}

/// On a computer with no Proton at all — made so here by hiding the system's, in an outer
/// bubblewrap, and giving the run an empty home — the run says what to install, and runs nothing.
#[test]
fn with_no_proton_at_all_the_run_says_what_to_install() {
    if !sandboxes() {
        return;
    }
    let home = scratch_home("no_proton");
    std::fs::create_dir_all(home.join("game")).unwrap();
    std::fs::write(home.join("game/game.exe"), "MZ").unwrap();
    let hidden = ["/usr/share/steam", "/usr/local/share/steam"]
        .into_iter()
        .filter(|dir| Path::new(dir).exists());
    let out = Command::new("bwrap")
        .args(["--dev-bind", "/", "/"])
        .args(hidden.flat_map(|dir| ["--tmpfs", dir]))
        .arg("--")
        .arg(env!("CARGO_BIN_EXE_win_ter"))
        .arg("--no-performance")
        .arg(home.join("game/game.exe"))
        .env("HOME", &home)
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("DISPLAY")
        .output()
        .expect("run bwrap");
    let said = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{said}");
    assert!(
        said.contains("win_ter: there's no Proton on this computer (Steam installs Valve's"),
        "{said}"
    );
    assert!(!home.join("Games").exists(), "nothing made for a run that can't happen");
    let _ = std::fs::remove_dir_all(&home);
}
