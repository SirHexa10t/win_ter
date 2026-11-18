//! The sandbox: bubblewrap's arguments for one run ([`bwrap_args`]), from what the run puts
//! together ([`Plan`]) and what this computer has to show ([`Host`]), and the sandbox's first step
//! ([`serve_filtered_if_asked`]) — the running program started again inside it, which sets the
//! syscall filter in place before Proton starts.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::prefix::PFX_DIR;
use crate::{proton, seccomp};

/// What one run puts together: what its lines are signed with, its Proton, the game and its
/// arguments, the game's own folder (the prefix), the folders it may write to, and whether it gets
/// the network and Wayland.
pub(crate) struct Plan<'a> {
    pub(crate) name: &'a str,
    pub(crate) proton: &'a Path,
    pub(crate) exe: &'a Path,
    pub(crate) game_dir: &'a Path,
    pub(crate) prefix: &'a Path,
    pub(crate) allowed: &'a [PathBuf],
    pub(crate) game_args: &'a [String],
    pub(crate) network: bool,
    pub(crate) wayland: bool,
}

/// What of this computer the sandbox shows the game, found by [`Host::here`]; apart from the
/// sandbox's arguments, so those can be built, and tested, for any computer.
#[derive(Debug, Default)]
pub(crate) struct Host {
    /// The top-level system folders shown read-only, each with its link's target where it is a
    /// link (`/bin` → `usr/bin`, as on merged-`/usr` systems).
    pub(crate) system: Vec<(PathBuf, Option<PathBuf>)>,
    /// The device nodes a game uses: GPUs, controllers, sound, and ntsync, Wine's fast sync.
    pub(crate) devices: Vec<PathBuf>,
    /// The user's runtime folder (`XDG_RUNTIME_DIR`), holding the display and sound sockets.
    pub(crate) runtime_dir: Option<PathBuf>,
    /// The Wayland display: its name as `WAYLAND_DISPLAY` gives it, and its socket.
    pub(crate) wayland: Option<(String, PathBuf)>,
    /// The X display (`DISPLAY`), and its authority file (`XAUTHORITY`) where there is one.
    pub(crate) x11: Option<(String, Option<PathBuf>)>,
    /// The sound servers' sockets: PipeWire's and PulseAudio's.
    pub(crate) sound: Vec<PathBuf>,
    /// udev's device database, through which controllers are found.
    pub(crate) udev: Option<PathBuf>,
    /// The folder `/etc/resolv.conf` links into, outside what's shown anyway (systemd-resolved
    /// keeps it in `/run`): name lookups, for a game given the network.
    pub(crate) resolver: Option<PathBuf>,
    /// The environment a game takes along: locale, timezone, and what tunes Proton and the graphics
    /// stack ([`passed_along`]).
    pub(crate) env: Vec<(String, OsString)>,
    /// Whether this bubblewrap lays a throwaway layer over a folder (`--tmp-overlay`, from version
    /// 0.8), so a Proton that writes into its own folder can, without changing it.
    pub(crate) overlays: bool,
    /// Whether this bubblewrap can keep the sandbox from making user namespaces of its own
    /// (`--disable-userns`): from version 0.8, and only where it isn't installed setuid.
    pub(crate) userns_locked: bool,
    /// The running program, put in the sandbox to set the syscall filter in place before Proton
    /// starts ([`serve_filtered_if_asked`]); none where there's no filter for this architecture.
    pub(crate) filter_exe: Option<PathBuf>,
}

impl Host {
    /// This computer, as the running user's session sees it.
    pub(crate) fn here() -> Self {
        let system = SYSTEM_DIRS
            .iter()
            .map(Path::new)
            .filter(|dir| dir.exists())
            .map(|dir| (dir.to_path_buf(), std::fs::read_link(dir).ok()))
            .collect();
        let mut devices: Vec<PathBuf> =
            DEVICES.iter().map(PathBuf::from).filter(|device| device.exists()).collect();
        if let Ok(entries) = std::fs::read_dir("/dev") {
            let numbered = |entry: &std::fs::DirEntry| {
                let name = entry.file_name();
                NUMBERED_DEVICES.iter().any(|kind| name.to_string_lossy().starts_with(kind))
            };
            let mut found: Vec<PathBuf> =
                entries.filter_map(Result::ok).filter(numbered).map(|entry| entry.path()).collect();
            found.sort();
            devices.extend(found);
        }
        let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute() && dir.is_dir());
        let wayland = std::env::var("WAYLAND_DISPLAY")
            .ok()
            .filter(|name| !name.is_empty())
            .and_then(|name| {
                let socket = match Path::new(&name).is_absolute() {
                    true => PathBuf::from(&name),
                    false => runtime_dir.as_ref()?.join(&name),
                };
                socket.exists().then_some((name, socket))
            });
        let x11 =
            std::env::var("DISPLAY").ok().filter(|display| !display.is_empty()).map(|display| {
                let authority =
                    std::env::var_os("XAUTHORITY").map(PathBuf::from).filter(|file| file.is_file());
                (display, authority)
            });
        let sound = runtime_dir
            .iter()
            .flat_map(|dir| SOUND_SOCKETS.iter().map(move |socket| dir.join(socket)))
            .filter(|socket| socket.exists())
            .collect();
        let resolver = std::fs::canonicalize("/etc/resolv.conf")
            .ok()
            .filter(|target| !target.starts_with("/etc") && !target.starts_with("/usr"))
            .and_then(|target| target.parent().map(Path::to_path_buf));
        let env = std::env::vars_os()
            .filter_map(|(key, value)| Some((key.into_string().ok()?, value)))
            .filter(|(key, _)| passed_along(key))
            .collect();
        let from_0_8 = stdout_of("bwrap", &["--version"]).is_some_and(|version| from_0_8(&version));
        Host {
            system,
            devices,
            runtime_dir,
            wayland,
            x11,
            sound,
            udev: Some(PathBuf::from("/run/udev")).filter(|udev| udev.is_dir()),
            resolver,
            env,
            overlays: from_0_8,
            userns_locked: from_0_8 && !setuid("bwrap"),
            filter_exe: seccomp::program(std::env::consts::ARCH)
                .and_then(|_| std::env::current_exe().ok()),
        }
    }
}

/// Whether the bubblewrap whose `--version` says `version` is 0.8 or later, which brought
/// throwaway overlays and `--disable-userns`.
fn from_0_8(version: &str) -> bool {
    let last = version.split_whitespace().last().unwrap_or("");
    let numbers: Vec<u32> = last.split('.').map_while(|part| part.parse().ok()).collect();
    matches!(numbers.as_slice(), [major, minor, ..] if (*major, *minor) >= (0, 8))
}

/// Whether `program`, as `PATH` finds it, is installed setuid.
fn setuid(program: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path_hits(program)
        .first()
        .and_then(|path| std::fs::metadata(path).ok())
        .is_some_and(|meta| meta.permissions().mode() & 0o4000 != 0)
}

/// Every executable `program` on `PATH`, in its order.
fn path_hits(program: &str) -> Vec<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .filter(|candidate| {
            std::fs::metadata(candidate)
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        })
        .collect()
}

/// Whether `program` is on `PATH`.
pub(crate) fn on_path(program: &str) -> bool {
    !path_hits(program).is_empty()
}

/// What `program` run with `args` prints, where it runs and succeeds.
fn stdout_of(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The sandbox's first step: the running program started again inside it
/// (`PROGRAM --proton-sandboxed-filtered NAME COMMAND ARGS…`), which sets the syscall filter in
/// place ([`seccomp`]) and becomes COMMAND — Proton's run, which the filter then holds for good;
/// NAME is what its lines are signed with. Returns `false` straight away where this process wasn't
/// started as that step; never returns otherwise. A filter that can't be set ends the process
/// there, so that no game runs unfiltered where a filter was meant.
///
/// A program embedding this crate hands its process to this before anything else.
pub fn serve_filtered_if_asked() -> bool {
    use std::os::unix::process::CommandExt;
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(OsStr::new(FILTERED_MARKER)) {
        return false;
    }
    let name = args.next().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let Some(command) = args.next() else {
        eprintln!("{name}: the sandbox was given nothing to run");
        std::process::exit(127);
    };
    let arch = std::env::consts::ARCH;
    let filter = seccomp::program(arch)
        .ok_or_else(|| std::io::Error::other(format!("there's none for {arch}")));
    if let Err(err) = filter.and_then(|filter| seccomp::install(&filter)) {
        eprintln!("{name}: couldn't set the syscall filter in place: {err}");
        std::process::exit(127);
    }
    let err = Command::new(&command).args(args).exec();
    eprintln!("{name}: couldn't start {}: {err}", command.to_string_lossy());
    std::process::exit(127);
}

/// The sandbox's arguments for `bwrap`, ending with what it runs. A namespace of its own for
/// everything, the network's included unless the game gets it, and IPC's unless it runs on X11,
/// whose shared memory would be cut off; the user's, in which no more can be made, where
/// bubblewrap can see to that. The system read-only, as it is here; devices, the display's and
/// sound's sockets; Proton under a throwaway layer (read-only, where bubblewrap has no such
/// layers); read-write the game's folder, the folders allowed it, and its own folder, the prefix.
/// A new session, so the game can't type into the terminal, and the sandbox goes when its starter
/// does. Then a clean environment, and [`RUN_SCRIPT`] in the .exe's folder — set going by the
/// running program, which sets the syscall filter in place first ([`serve_filtered_if_asked`]),
/// where there's a filter for this architecture.
pub(crate) fn bwrap_args(plan: &Plan, host: &Host) -> Vec<OsString> {
    let mut args: Vec<OsString> = Vec::new();
    let mut add = |words: &[&OsStr]| args.extend(words.iter().map(|word| word.to_os_string()));
    let namespaces = ["--die-with-parent", "--new-session", "--unshare-pid", "--unshare-uts"];
    add(&namespaces.map(OsStr::new));
    add(&["--unshare-cgroup-try".as_ref()]);
    if host.userns_locked {
        add(&["--unshare-user", "--disable-userns"].map(OsStr::new));
    }
    if plan.wayland {
        add(&["--unshare-ipc".as_ref()]);
    }
    if !plan.network {
        add(&["--unshare-net".as_ref()]);
    }
    for (dir, link) in &host.system {
        match link {
            Some(target) => add(&["--symlink".as_ref(), target.as_os_str(), dir.as_os_str()]),
            None => add(&["--ro-bind".as_ref(), dir.as_os_str(), dir.as_os_str()]),
        }
    }
    add(&["--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp"].map(OsStr::new));
    for device in &host.devices {
        add(&["--dev-bind".as_ref(), device.as_os_str(), device.as_os_str()]);
    }
    let read_only =
        |path: &Path| -> [OsString; 3] { ["--ro-bind".into(), path.into(), path.into()] };
    let mut shown: Vec<&Path> = host.udev.iter().map(PathBuf::as_path).collect();
    if plan.network {
        shown.extend(host.resolver.as_deref());
    }
    if let Some(runtime) = &host.runtime_dir {
        add(&["--perms".as_ref(), "0700".as_ref(), "--dir".as_ref(), runtime.as_os_str()]);
    }
    match (&host.wayland, &host.x11) {
        (Some((_, socket)), _) if plan.wayland => shown.push(socket),
        (_, Some((_, authority))) if !plan.wayland => {
            shown.push(Path::new(X11_SOCKETS));
            shown.extend(authority.as_deref());
        }
        _ => {}
    }
    shown.extend(host.sound.iter().map(PathBuf::as_path));
    for path in shown {
        args.extend(read_only(path));
    }
    // Valve's Protons write into their own folder (a lock, fixes after Steam's updates, their
    // default prefix): given a throwaway layer, they can, and what they write goes with the
    // sandbox. Never the folder itself: a game that changed Proton would reach every game it runs,
    // the unsandboxed ones too.
    match host.overlays {
        true => {
            args.extend(["--overlay-src".into(), plan.proton.into()]);
            args.extend(["--tmp-overlay".into(), plan.proton.into()]);
        }
        false => args.extend(read_only(plan.proton)),
    }
    if let Some(filter) = &host.filter_exe {
        args.extend(["--ro-bind".into(), filter.into(), FILTER_IN_SANDBOX.into()]);
    }
    let writable = std::iter::once(plan.game_dir)
        .chain(plan.allowed.iter().map(PathBuf::as_path))
        .chain([plan.prefix]);
    for dir in writable {
        args.extend(["--bind".into(), dir.into(), dir.into()]);
    }
    args.extend(["--chdir".into(), plan.exe.parent().unwrap_or(plan.game_dir).into()]);
    args.push("--clearenv".into());
    for (key, value) in sandbox_env(plan, host) {
        args.extend(["--setenv".into(), key.into(), value]);
    }
    args.push("--".into());
    if host.filter_exe.is_some() {
        args.extend([FILTER_IN_SANDBOX.into(), FILTERED_MARKER.into(), plan.name.into()]);
    }
    args.extend(["/bin/sh".into(), "-c".into(), RUN_SCRIPT.into(), "sh".into()]);
    args.extend([plan.proton.into(), plan.exe.into()]);
    args.extend(plan.game_args.iter().map(OsString::from));
    args
}

/// The environment inside the sandbox. What the computer passes along ([`Host::env`]), the
/// session's display and runtime folder, then what Proton needs outside Steam: its data folder
/// (the prefix), a Steam client folder (an empty one: there is no Steam in the sandbox), the game's
/// folder, the home the game sees. The Wayland driver switched on where it's used, and the shader
/// caches kept whole, so what was compiled once isn't thrown away and compiled again mid-game (the
/// user's own settings for them win).
fn sandbox_env(plan: &Plan, host: &Host) -> Vec<(String, OsString)> {
    let mut env = host.env.clone();
    let mut set = |key: &str, value: OsString| {
        env.retain(|(known, _)| known != key);
        env.push((key.to_string(), value));
    };
    set("HOME", plan.prefix.join(HOME_DIR).into());
    set("PATH", "/usr/local/bin:/usr/bin:/bin".into());
    set("STEAM_COMPAT_DATA_PATH", plan.prefix.into());
    set("STEAM_COMPAT_CLIENT_INSTALL_PATH", plan.prefix.join(STEAM_CLIENT_DIR).into());
    set("STEAM_COMPAT_INSTALL_PATH", plan.game_dir.into());
    set("WINEPREFIX", plan.prefix.join(PFX_DIR).into());
    if let Some(runtime) = &host.runtime_dir {
        set("XDG_RUNTIME_DIR", runtime.into());
    }
    match (&host.wayland, &host.x11) {
        (Some((name, _)), _) if plan.wayland => {
            set("WAYLAND_DISPLAY", name.into());
            set(proton::WAYLAND_SWITCH, "1".into());
        }
        (_, Some((display, authority))) if !plan.wayland => {
            set("DISPLAY", display.into());
            if let Some(authority) = authority {
                set("XAUTHORITY", authority.into());
            }
        }
        _ => {}
    }
    for &(key, value) in SHADER_CACHES {
        if !env.iter().any(|(known, _)| known == key) {
            env.push((key.to_string(), value.into()));
        }
    }
    env
}

/// Whether the variable `key` goes into the sandbox: the locale and timezone, the user's name, and
/// the variables that tune Proton, Wine and the graphics stack, as the user set them — but not
/// what the run sets itself (the prefix, the Wayland switch).
fn passed_along(key: &str) -> bool {
    let tuning = PASSED_PREFIXES.iter().any(|prefix| key.starts_with(prefix));
    (PASSED_ALONG.contains(&key) || tuning) && !NOT_PASSED.contains(&key)
}

/// What the sandbox runs, `$1` Proton's folder and the rest the game and its arguments: Proton's
/// `waitforexitandrun`, the verb Steam uses (it lets the prefix's previous session end, then runs
/// the game), then `wineserver -w`, so Wine's server ends on its own and saves the prefix's
/// registry, which closing the sandbox under it would lose. The game's exit status is the run's.
pub(crate) const RUN_SCRIPT: &str = r#"proton=$1; shift; "$proton/proton" waitforexitandrun "$@"; status=$?; "$proton/files/bin/wineserver" -w 2>/dev/null; exit $status"#;

/// Where the running program is put in the sandbox, read-only, to set the syscall filter in place.
const FILTER_IN_SANDBOX: &str = "/run/proton_sandboxed";
/// What the running program is started with in the sandbox to set the syscall filter in place and
/// run the rest of its arguments ([`serve_filtered_if_asked`]): a marker no command uses.
const FILTERED_MARKER: &str = "--proton-sandboxed-filtered";
/// The home the game sees, in its own folder.
pub(crate) const HOME_DIR: &str = "home";
/// The Steam client folder Proton is told of, in the game's own folder: empty, as there's no Steam
/// in the sandbox.
pub(crate) const STEAM_CLIENT_DIR: &str = "steam";
/// Where Proton's script finds python3, inside the sandbox.
pub(crate) const SYSTEM_PYTHONS: &[&str] = &["/usr/bin/python3", "/bin/python3"];
/// The system folders shown read-only (where they exist): links are kept as links.
const SYSTEM_DIRS: &[&str] = &["/usr", "/etc", "/bin", "/sbin", "/lib", "/lib32", "/lib64", "/sys"];
/// The device folders and nodes shown: GPUs, controllers, sound, and ntsync.
const DEVICES: &[&str] = &["/dev/dri", "/dev/input", "/dev/snd", "/dev/ntsync"];
/// The devices in `/dev` shown by their name's start: NVIDIA's GPU nodes, and the raw HID nodes
/// controllers are read through.
const NUMBERED_DEVICES: &[&str] = &["nvidia", "hidraw"];
/// The sound servers' sockets in the runtime folder: PipeWire's, and PulseAudio's (which PipeWire
/// serves as well).
const SOUND_SOCKETS: &[&str] = &["pipewire-0", "pulse/native"];
/// The X server's sockets.
const X11_SOCKETS: &str = "/tmp/.X11-unix";
/// The variables passed into the sandbox by name.
const PASSED_ALONG: &[&str] = &["LANG", "LANGUAGE", "TZ", "USER", "LOGNAME"];
/// The variables passed into the sandbox by their name's start: the locale's, and those that tune
/// Proton, Wine, DXVK and VKD3D, the GPU drivers and Vulkan, SDL, and MangoHud.
const PASSED_PREFIXES: &[&str] = &[
    "LC_", "PROTON_", "WINE", "DXVK_", "VKD3D_", "__GL_", "MESA_", "RADV_", "VK_", "SDL_",
    "MANGOHUD",
];
/// The variables the run sets itself, which a passed-along one mustn't override.
const NOT_PASSED: &[&str] = &["WINEPREFIX", "PROTON_ENABLE_WAYLAND", "PROTON_USE_WAYLAND"];
/// The shader caches' settings, set where the user hasn't: NVIDIA's cache isn't trimmed to its size
/// limit, and Mesa's may grow to 10 GB.
const SHADER_CACHES: &[(&str, &str)] =
    &[("__GL_SHADER_DISK_CACHE_SKIP_CLEANUP", "1"), ("MESA_SHADER_CACHE_MAX_SIZE", "10G")];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proton::tests::scratch;

    /// A computer with every kind of thing to show, for the sandbox's arguments.
    fn host() -> Host {
        Host {
            system: vec![
                (PathBuf::from("/usr"), None),
                (PathBuf::from("/bin"), Some(PathBuf::from("usr/bin"))),
            ],
            devices: vec![PathBuf::from("/dev/dri"), PathBuf::from("/dev/ntsync")],
            runtime_dir: Some(PathBuf::from("/run/user/1000")),
            wayland: Some(("wayland-0".into(), PathBuf::from("/run/user/1000/wayland-0"))),
            x11: Some((":0".into(), Some(PathBuf::from("/run/user/1000/xauth_abc")))),
            sound: vec![PathBuf::from("/run/user/1000/pipewire-0")],
            udev: Some(PathBuf::from("/run/udev")),
            resolver: Some(PathBuf::from("/run/systemd/resolve")),
            env: vec![
                ("LANG".into(), "en_US.UTF-8".into()),
                ("DXVK_HUD".into(), "fps".into()),
                ("MESA_SHADER_CACHE_MAX_SIZE".into(), "2G".into()),
            ],
            overlays: true,
            userns_locked: true,
            filter_exe: Some(PathBuf::from("/opt/runner/runner")),
        }
    }

    /// Throwaway layers and `--disable-userns` came with bubblewrap 0.8.
    #[test]
    fn overlays_and_the_userns_lock_come_with_bubblewrap_0_8() {
        assert!(from_0_8("bubblewrap 0.13.1\n"));
        assert!(from_0_8("bubblewrap 0.8.0"));
        assert!(from_0_8("bubblewrap 1.0"));
        assert!(!from_0_8("bubblewrap 0.6.1"));
        assert!(!from_0_8("nonsense"));
    }

    /// The arguments as text, to search through.
    fn words(args: &[OsString]) -> Vec<String> {
        args.iter().map(|arg| arg.to_string_lossy().into_owned()).collect()
    }

    /// Whether `seq` appears in `args`, in order and side by side.
    fn has(args: &[String], seq: &[&str]) -> bool {
        args.windows(seq.len()).any(|window| window.iter().zip(seq).all(|(arg, want)| arg == want))
    }

    fn plan<'a>(
        network: bool,
        wayland: bool,
        allowed: &'a [PathBuf],
        game_args: &'a [String],
    ) -> Plan<'a> {
        Plan {
            name: "runner",
            proton: Path::new("/protons/cachy"),
            exe: Path::new("/games/G/bin/g.exe"),
            game_dir: Path::new("/games/G"),
            prefix: Path::new("/home/u/Games/manual_run_proton_prefixes/G-1"),
            allowed,
            game_args,
            network,
            wayland,
        }
    }

    /// The sandbox by default: no network, its own IPC, a user namespace that can make no more, the
    /// system read-only with its links as links, devices, the Wayland socket (no X11), sound,
    /// Proton read-only, the game's folders read-write, run from the .exe's folder with a clean
    /// environment by the running program, which sets the syscall filter in place first, the
    /// game's arguments last, untouched.
    #[test]
    fn the_sandbox_shows_the_system_read_only_and_lets_the_game_write_only_its_folders() {
        let game_args = ["-windowed".to_string(), "--level 2".to_string()];
        let args = words(&bwrap_args(&plan(false, true, &[], &game_args), &host()));
        for seq in [
            &["--unshare-user", "--disable-userns"][..],
            &["--ro-bind", "/opt/runner/runner", FILTER_IN_SANDBOX],
            &["--unshare-net"],
            &["--unshare-ipc"],
            &["--new-session"],
            &["--die-with-parent"],
            &["--ro-bind", "/usr", "/usr"],
            &["--symlink", "usr/bin", "/bin"],
            &["--tmpfs", "/tmp"],
            &["--dev-bind", "/dev/dri", "/dev/dri"],
            &["--dev-bind", "/dev/ntsync", "/dev/ntsync"],
            &["--ro-bind", "/run/udev", "/run/udev"],
            &["--perms", "0700", "--dir", "/run/user/1000"],
            &["--ro-bind", "/run/user/1000/wayland-0", "/run/user/1000/wayland-0"],
            &["--ro-bind", "/run/user/1000/pipewire-0", "/run/user/1000/pipewire-0"],
            &["--overlay-src", "/protons/cachy", "--tmp-overlay", "/protons/cachy"],
            &["--bind", "/games/G", "/games/G"],
            &[
                "--bind",
                "/home/u/Games/manual_run_proton_prefixes/G-1",
                "/home/u/Games/manual_run_proton_prefixes/G-1",
            ],
            &["--chdir", "/games/G/bin", "--clearenv"],
            &["--setenv", "WAYLAND_DISPLAY", "wayland-0"],
            &["--setenv", "PROTON_ENABLE_WAYLAND", "1"],
            &[
                "--",
                FILTER_IN_SANDBOX,
                FILTERED_MARKER,
                "runner",
                "/bin/sh",
                "-c",
                RUN_SCRIPT,
                "sh",
                "/protons/cachy",
                "/games/G/bin/g.exe",
                "-windowed",
                "--level 2",
            ],
        ] {
            assert!(has(&args, seq), "{seq:?} in {args:?}");
        }
        assert!(args.last().is_some_and(|last| last == "--level 2"));
        for absent in [
            X11_SOCKETS,
            "/run/user/1000/xauth_abc",
            "DISPLAY",
            "/run/systemd/resolve",
            "--share-net",
        ] {
            assert!(!args.iter().any(|arg| arg == absent), "{absent} has no place here: {args:?}");
        }
        let at = |seq: &[&str]| {
            args.windows(seq.len())
                .position(|window| window.iter().zip(seq).all(|(arg, want)| arg == want))
                .unwrap()
        };
        assert!(
            at(&["--tmpfs", "/tmp"]) < at(&["--perms"]),
            "the runtime folder after the fresh /tmp"
        );
        assert!(
            at(&["--clearenv"]) < at(&["--setenv", "HOME"]),
            "the environment cleared before it's set"
        );
        assert!(
            !has(&args, &["--ro-bind", "/protons/cachy", "/protons/cachy"])
                && !has(&args, &["--bind", "/protons/cachy", "/protons/cachy"])
        );

        let older = Host { overlays: false, userns_locked: false, filter_exe: None, ..host() };
        let args = words(&bwrap_args(&plan(false, true, &[], &game_args), &older));
        assert!(
            has(&args, &["--ro-bind", "/protons/cachy", "/protons/cachy"]),
            "read-only where there are no throwaway layers"
        );
        assert!(
            has(&args, &["--", "/bin/sh", "-c", RUN_SCRIPT]),
            "no filter to set where there's none for the architecture"
        );
        for absent in [
            "--tmp-overlay",
            "--disable-userns",
            "--unshare-user",
            FILTER_IN_SANDBOX,
            FILTERED_MARKER,
        ] {
            assert!(!args.iter().any(|arg| arg == absent), "{absent} has no place here: {args:?}");
        }
    }

    /// With the network, the resolver's folder is shown too; on X11, the X sockets (after the fresh
    /// `/tmp`) and the authority file, the host's IPC kept, and no Wayland at all; and the folders
    /// allowed are writable.
    #[test]
    fn the_network_and_x11_add_what_they_need_and_nothing_more() {
        let allowed = [PathBuf::from("/mnt/mods")];
        let args = words(&bwrap_args(&plan(true, false, &allowed, &[]), &host()));
        assert!(has(&args, &["--ro-bind", "/run/systemd/resolve", "/run/systemd/resolve"]));
        assert!(has(&args, &["--ro-bind", X11_SOCKETS, X11_SOCKETS]));
        assert!(has(&args, &["--ro-bind", "/run/user/1000/xauth_abc", "/run/user/1000/xauth_abc"]));
        assert!(has(&args, &["--setenv", "DISPLAY", ":0"]));
        assert!(has(&args, &["--bind", "/mnt/mods", "/mnt/mods"]));
        for absent in [
            "--unshare-net",
            "--unshare-ipc",
            "/run/user/1000/wayland-0",
            "WAYLAND_DISPLAY",
            "PROTON_ENABLE_WAYLAND",
        ] {
            assert!(!args.iter().any(|arg| arg == absent), "{absent} has no place here: {args:?}");
        }
        let at = |word: &str| args.iter().position(|arg| arg == word).unwrap();
        assert!(at("/tmp") < at(X11_SOCKETS));
    }

    /// The environment inside: Proton's folders, the home the game sees, the user's tuning passed
    /// along (their shader-cache size winning over the default), and the sync and cache settings.
    #[test]
    fn the_sandbox_environment_carries_protons_folders_and_the_users_tuning() {
        let env = sandbox_env(&plan(false, true, &[], &[]), &host());
        let get = |key: &str| {
            env.iter()
                .find(|(known, _)| known == key)
                .map(|(_, value)| value.to_string_lossy().into_owned())
        };
        let prefix = "/home/u/Games/manual_run_proton_prefixes/G-1";
        assert_eq!(get("HOME"), Some(format!("{prefix}/home")));
        assert_eq!(get("STEAM_COMPAT_DATA_PATH").as_deref(), Some(prefix));
        assert_eq!(get("STEAM_COMPAT_CLIENT_INSTALL_PATH"), Some(format!("{prefix}/steam")));
        assert_eq!(get("STEAM_COMPAT_INSTALL_PATH").as_deref(), Some("/games/G"));
        assert_eq!(get("WINEPREFIX"), Some(format!("{prefix}/pfx")));
        assert_eq!(get("XDG_RUNTIME_DIR").as_deref(), Some("/run/user/1000"));
        assert_eq!(get("DXVK_HUD").as_deref(), Some("fps"));
        assert_eq!(get("LANG").as_deref(), Some("en_US.UTF-8"));
        assert_eq!(get("MESA_SHADER_CACHE_MAX_SIZE").as_deref(), Some("2G"), "the user's own wins");
        assert_eq!(get("__GL_SHADER_DISK_CACHE_SKIP_CLEANUP").as_deref(), Some("1"));
        assert_eq!(env.iter().filter(|(key, _)| key == "HOME").count(), 1);
    }

    /// What goes in from the user's environment: locale, timezone, user name and tuning — not what
    /// the run sets itself, nor anything else of the session.
    #[test]
    fn only_locale_and_tuning_variables_are_passed_along() {
        for passed in [
            "LANG",
            "LC_ALL",
            "TZ",
            "PROTON_LOG",
            "DXVK_HUD",
            "VKD3D_CONFIG",
            "WINEDEBUG",
            "MANGOHUD",
            "MANGOHUD_CONFIG",
            "__GL_SYNC_TO_VBLANK",
            "SDL_GAMECONTROLLERCONFIG",
        ] {
            assert!(passed_along(passed), "{passed}");
        }
        for kept_out in [
            "WINEPREFIX",
            "PROTON_ENABLE_WAYLAND",
            "PROTON_USE_WAYLAND",
            "SSH_AUTH_SOCK",
            "DBUS_SESSION_BUS_ADDRESS",
            "HOME",
            "PATH",
            "GNUPGHOME",
            "DISPLAY",
        ] {
            assert!(!passed_along(kept_out), "{kept_out}");
        }
    }

    /// The run script runs Proton's verb with the game and its arguments as given, waits for Wine's
    /// server, and exits with the game's status. Run here by `sh` against a stand-in.
    #[test]
    fn the_run_script_passes_everything_and_keeps_the_games_status() {
        let base = scratch("script");
        let proton = base.join("proton-dir");
        std::fs::create_dir_all(proton.join("files/bin")).unwrap();
        let write = |path: PathBuf, body: &str| {
            std::fs::write(&path, body).unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        let log = base.join("log");
        write(
            proton.join("proton"),
            &format!("#!/bin/sh\nprintf '%s|' \"$@\" >> '{}'; exit 3\n", log.display()),
        );
        write(
            proton.join("files/bin/wineserver"),
            &format!("#!/bin/sh\necho \" wineserver $*\" >> '{}'\n", log.display()),
        );
        let status = Command::new("/bin/sh")
            .args(["-c", RUN_SCRIPT, "sh"])
            .arg(&proton)
            .args(["/g/game.exe", "-a b", "--c"])
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(3), "the game's status");
        assert_eq!(
            std::fs::read_to_string(&log).unwrap(),
            "waitforexitandrun|/g/game.exe|-a b|--c| wineserver -w\n"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// `PATH` is searched for executables only, in its order.
    #[test]
    fn programs_are_found_on_path() {
        assert!(on_path("sh"));
        assert!(!on_path("no-such-program-anywhere"));
        assert!(path_hits("sh").iter().all(|hit| hit.ends_with("sh")));
    }
}
