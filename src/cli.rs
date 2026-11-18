//! The command line, as a library: all the `win_ter` binary is, and what a program
//! embedding it reuses rather than restating the flags.
//!
//! A run's arguments are [`RunArgs`]; [`run`] does the run and returns its exit status as a plain
//! number — the game's, or 1 where the run was refused. What the program running it brings along
//! (the name its lines are signed with, a Proton of its own, a word when the game starts) is an
//! [`Embedding`]; running on its own, it is [`Embedding::standalone`].

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::{Args, Parser};

use crate::prefix::{self, PastPrefix};
use crate::sandbox::{self, Host, Plan};
use crate::{folders, game, proton};

/// Run a Windows game (.exe) through Proton, in a sandbox that sees only its own folders
///
/// The newest Proton here runs it, on Wine's own Wayland driver where that Proton has one, and with
/// the network cut off. The sandbox (bubblewrap) shows the system read-only, and lets the game write
/// only to its own folder and to one of its own under ~/Games/manual_run_proton_prefixes: its Wine
/// prefix, where Windows games keep their saves, and the home it sees. Neither may be, or hold, one
/// of your own folders (your home, Downloads, …) or a whole filesystem, nor may a folder --allow
/// gives it. Inside, a syscall filter (Flatpak's) refuses the calls a game has no use for, and the
/// game can make no namespaces of its own. The GPU, controllers and sound reach it. Where CachyOS's
/// game-performance (or Feral's GameMode) is installed, the game runs under it. A game another
/// launcher has run (Steam, so far) can go on from that launcher's prefix: --prefix-continue runs it
/// there, --take-ownership copies it into the game's own folder first. Options go before the .exe or
/// after it: the game's own arguments start at the first word after it that isn't one — or after
/// --, which keeps them from being read as options.
#[derive(Parser, Debug)]
#[command(name = "win_ter", version)]
pub struct Cli {
    #[command(flatten)]
    pub run: RunArgs,
}

impl Cli {
    /// Run the game, on its own ([`Embedding::standalone`]); the exit status, as [`run`] gives it.
    pub fn run(self) -> u8 {
        run(self.run, &Embedding::standalone())
    }
}

/// The game, its arguments, and how to run it.
#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct RunArgs {
    /// The game's .exe, or the game's folder where that holds just one (crash handlers,
    /// uninstallers and the like aside)
    pub exe: PathBuf,
    /// Arguments for the game
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub game_args: Vec<String>,
    /// Run on X11 (XWayland) instead of Wine's own Wayland driver: for a game that misbehaves on
    /// Wayland, or to take the newest Proton when it has no Wayland driver
    #[arg(long)]
    pub xwayland: bool,
    /// Let the game reach the network, which it is cut off from by default
    #[arg(long)]
    pub network: bool,
    /// The Proton to run it on instead of the newest: its name (or part of it), or its folder
    #[arg(long, value_name = "NAME|DIR")]
    pub proton: Option<String>,
    /// The game's folder, which it may write to, when the .exe sits deeper in it (bin/,
    /// Binaries/Win64/); the .exe's own folder by default
    #[arg(long, value_name = "DIR")]
    pub game_dir: Option<PathBuf>,
    /// One more folder the game may read and write; may be given again
    #[arg(long, value_name = "DIR")]
    pub allow: Vec<PathBuf>,
    /// The game's own folder (its Wine prefix and the home it sees), instead of the one made for it
    /// under ~/Games/manual_run_proton_prefixes; a Steam game's is its folder in Steam's
    /// steamapps/compatdata
    #[arg(long, value_name = "DIR")]
    pub prefix: Option<PathBuf>,
    /// Run the game on the prefix another launcher made for this .exe (Steam's, so far), where it
    /// is, as --prefix with its folder would. Refused where no launcher has a prefix for it; where
    /// several have, a numbered menu asks which
    #[arg(long, conflicts_with = "prefix")]
    pub prefix_continue: bool,
    /// Copy the prefix another launcher made for this .exe into the game's own folder (the default
    /// one, or --prefix), then run the game there: found as --prefix-continue finds it, with the
    /// same menu and the same refusals, and refused where that folder is already there
    #[arg(long, conflicts_with = "prefix_continue")]
    pub take_ownership: bool,
    /// Run without game-performance or GameMode, even where installed
    #[arg(long)]
    pub no_performance: bool,
    /// Print the command that would run, and run nothing
    #[arg(long)]
    pub dry_run: bool,
}

impl RunArgs {
    /// The words that run it again, after a program's name — in one order, however it was typed:
    /// the .exe first, so that sorted, the runs of a game and of the games beside it sit together;
    /// then the options it has, in the order they are declared above (`--help`'s); then the game's
    /// arguments, after `--` where the first starts with `-` and would be read as an option. A .exe
    /// whose path starts with `-` is written from `./`, for the same reason.
    pub fn words(&self) -> Vec<OsString> {
        let exe = match self.exe.as_os_str().as_encoded_bytes().starts_with(b"-") {
            true => Path::new(".").join(&self.exe),
            false => self.exe.clone(),
        };
        let switch = |on: bool, flag: &str| on.then(|| vec![OsString::from(flag)]);
        let valued = |flag: &str, value: &OsStr| vec![OsString::from(flag), value.to_os_string()];
        let options = [
            switch(self.xwayland, "--xwayland"),
            switch(self.network, "--network"),
            self.proton.as_ref().map(|proton| valued("--proton", proton.as_ref())),
            self.game_dir.as_ref().map(|dir| valued("--game-dir", dir.as_ref())),
            Some(self.allow.iter().flat_map(|dir| valued("--allow", dir.as_ref())).collect()),
            self.prefix.as_ref().map(|dir| valued("--prefix", dir.as_ref())),
            switch(self.prefix_continue, "--prefix-continue"),
            switch(self.take_ownership, "--take-ownership"),
            switch(self.no_performance, "--no-performance"),
            switch(self.dry_run, "--dry-run"),
        ];
        let mut words = vec![exe.into_os_string()];
        words.extend(options.into_iter().flatten().flatten());
        if self.game_args.first().is_some_and(|arg| arg.starts_with('-')) {
            words.push("--".into());
        }
        words.extend(self.game_args.iter().map(OsString::from));
        words
    }
}

/// What a program running win_ter brings to a run.
pub struct Embedding<'a> {
    /// What the run's lines are signed with: the command's name.
    pub name: &'a str,
    /// The folder of a Proton the program keeps itself, looked for beside the system's (it needn't
    /// be there yet).
    pub own_proton: Option<&'a Path>,
    /// Whether that Proton is taken whatever newer ones there are.
    pub own_proton_first: bool,
    /// Fetches a Proton into [`own_proton`](Self::own_proton) where the computer has none at all,
    /// saying first what it does; why not, where it can't. Without one, a computer with no Proton is
    /// told what to install.
    pub fetch_proton: Option<&'a dyn Fn() -> Result<(), String>>,
    /// Folders kept off `PATH` for the performance wrapper: the program's own stand-ins for tools,
    /// which the wrapper's helpers mustn't take for the system's (`game-performance` runs the
    /// system's python scripts, needing the system's python).
    pub off_path: &'a [PathBuf],
    /// Told that the game is about to start, once a run is ready — never for a dry run, nor a
    /// refused one — with the run's arguments as it takes them, settled ([`as_run`]): to run it
    /// again from anywhere, as [`RunArgs::words`] spells it.
    pub on_start: Option<&'a dyn Fn(&RunArgs)>,
}

impl Embedding<'static> {
    /// Running on its own: lines signed `win_ter`, the system's Protons alone.
    pub fn standalone() -> Self {
        Embedding {
            name: "win_ter",
            own_proton: None,
            own_proton_first: false,
            fetch_proton: None,
            off_path: &[],
            on_start: None,
        }
    }
}

/// Run the game as `args` say: the game's exit status, or 1 where the run is refused, saying why
/// on stderr.
pub fn run(args: RunArgs, embedding: &Embedding) -> u8 {
    launch(&args, embedding).unwrap_or_else(|why| {
        eprintln!("{}: {why}", embedding.name);
        1
    })
}

/// The run: the game and its folders, checked; its own folder, or another launcher's prefix;
/// the Proton it takes; then the sandbox's command — printed, for a dry run, or run.
fn launch(args: &RunArgs, embedding: &Embedding) -> Result<u8, String> {
    let name = embedding.name;
    let home = crate::home();
    let exe = match std::fs::canonicalize(&args.exe) {
        Ok(folder) if folder.is_dir() => game::exe_in(&folder)?,
        Ok(exe) if exe.is_file() => exe,
        _ => return Err(format!("{} is no file", args.exe.display())),
    };
    let game_dir = match &args.game_dir {
        Some(dir) => {
            std::fs::canonicalize(dir).map_err(|_| format!("{} is no folder", dir.display()))?
        }
        None => exe.parent().map(Path::to_path_buf).unwrap_or_default(),
    };
    if !exe.starts_with(&game_dir) {
        return Err(format!(
            "{} isn't inside the game's folder, {}",
            exe.display(),
            game_dir.display()
        ));
    }
    let allowed = args
        .allow
        .iter()
        .map(|dir| {
            std::fs::canonicalize(dir).map_err(|_| format!("{} is no folder", dir.display()))
        })
        .collect::<Result<Vec<PathBuf>, String>>()?;
    // No folder the game may write to may be, or hold, the user's own folders (the home,
    // Downloads, …) or a whole filesystem: the game could write to all of it.
    let own = folders::users_folders(&crate::canonical(&home), &|key| std::env::var_os(key));
    let mounts = folders::mount_points();
    if let Some(why) = folders::too_broad(&game_dir, &own, &mounts) {
        let remedy = match args.game_dir {
            Some(_) => "name a folder that is the game's own with --game-dir",
            None => {
                "put the .exe in a folder of its own, or name the game's folder with --game-dir"
            }
        };
        return Err(format!(
            "the game could write to all of its folder, {}, and {why} ({remedy})",
            game_dir.display()
        ));
    }
    for dir in &allowed {
        if let Some(why) = folders::too_broad(dir, &own, &mounts) {
            return Err(format!(
                "--allow {}: the game could write to all of it, and {why} (allow a folder inside it instead)",
                dir.display()
            ));
        }
    }
    if !sandbox::on_path("bwrap") {
        return Err("its sandbox is bubblewrap (bwrap), which isn't installed".to_string());
    }
    if !sandbox::SYSTEM_PYTHONS.iter().any(|python| Path::new(python).is_file()) {
        return Err("Proton runs on the system's python3, which isn't installed".to_string());
    }
    // The game's own folder; or, to go on from another launcher's, that one — where it is, or
    // copied into the game's own folder, which mustn't be there yet. Settled before a Proton is
    // fetched, so that nothing comes down for a run that can't happen.
    let given = args
        .prefix
        .as_deref()
        .map(|dir| std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf()));
    let own_folder = given.unwrap_or_else(|| prefix::own_folder(&home, &game_dir));
    if let Some(why) =
        folders::too_broad(&own_folder, &own, &mounts).filter(|_| !args.prefix_continue)
    {
        return Err(format!(
            "--prefix {}: the game could write to all of it, and {why} (name a folder that is the game's own)",
            own_folder.display()
        ));
    }
    if args.take_ownership && std::fs::symlink_metadata(&own_folder).is_ok() {
        return Err(format!(
            "{} is already there: the game has a prefix of its own (move it away to take over another)",
            own_folder.display()
        ));
    }
    let past = match args.take_ownership || args.prefix_continue {
        true => Some(past_prefix(&exe, &home)?),
        false => None,
    };
    let prefix = match &past {
        Some(past) if args.prefix_continue => past.folder.clone(),
        _ => own_folder,
    };

    let mut protons = proton::installed(&home, embedding.own_proton);
    // A Proton asked for by its folder needn't be where Steam keeps them.
    let asked =
        args.proton.as_deref().and_then(|asked| proton::at(Path::new(asked), embedding.own_proton));
    if let Some(asked) = asked {
        if !protons.iter().any(|known| crate::canonical(&known.dir) == crate::canonical(&asked.dir))
        {
            protons.push(asked);
        }
    }
    if protons.is_empty() && args.proton.is_none() {
        let fetch = embedding.fetch_proton.ok_or(
            "there's no Proton on this computer (Steam installs Valve's; proton-cachyos and GE-Proton come as packages)",
        )?;
        fetch().map_err(|why| {
            format!("no Proton on this computer, and none could be fetched: {why}")
        })?;
        protons = proton::installed(&home, embedding.own_proton);
    }
    let host = Host::here();
    let want_wayland = !args.xwayland && host.wayland.is_some();
    let choice =
        proton::choose(&protons, args.proton.as_deref(), want_wayland, embedding.own_proton_first)?;
    let plan = Plan {
        name,
        proton: &choice.proton.dir,
        exe: &exe,
        game_dir: &game_dir,
        prefix: &prefix,
        allowed: &allowed,
        game_args: &args.game_args,
        network: args.network,
        wayland: choice.wayland,
    };
    let mut command: Vec<OsString> = Vec::new();
    if !args.no_performance {
        command.extend(performance_wrapper().map(OsString::from));
    }
    command.push("bwrap".into());
    command.extend(sandbox::bwrap_args(&plan, &host));

    if args.dry_run {
        if let Some(past) = past.as_ref().filter(|_| args.take_ownership) {
            eprintln!("{name}: would copy {} into {} first", past.described(), prefix.display());
        }
        let words: Vec<String> = command.iter().map(|word| quoted(word)).collect();
        println!("{}", words.join(" "));
        return Ok(0);
    }
    if let Some(past) = &past {
        if args.take_ownership {
            eprintln!("{name}: copying {} into {}", past.described(), prefix.display());
            prefix::copy(&past.folder, &prefix)?;
        } else {
            eprintln!("{name}: going on from {}, where it is", past.described());
        }
    }
    for dir in [prefix.join(sandbox::HOME_DIR), prefix.join(sandbox::STEAM_CLIENT_DIR)] {
        std::fs::create_dir_all(&dir)
            .map_err(|err| format!("couldn't make {}: {err}", dir.display()))?;
    }
    eprintln!("{name}: {}", summary(name, &choice, &plan, &host, args.xwayland));
    if let Some(on_start) = embedding.on_start {
        on_start(&as_run(args, &exe, &game_dir, &allowed, &prefix));
    }
    let status = Command::new(&command[0])
        .args(&command[1..])
        .env("PATH", path_without(embedding.off_path))
        .status()
        .map_err(|err| format!("couldn't start {}: {err}", command[0].to_string_lossy()))?;
    Ok(status.code().and_then(|code| u8::try_from(code).ok()).unwrap_or(1))
}

/// The run's arguments as it takes them, settled — its .exe, its game's folder, the folders allowed
/// it and its own folder — to run it again from anywhere: each path whole, links followed (the .exe
/// the one found, where its folder was given), and a Proton named by its folder, that folder so.
/// What wasn't given stays so, and the rest is as given: `--take-ownership` too, though its copy is
/// there by then, so that the same run again would be refused (a program keeping runs to repeat
/// leaves it off).
fn as_run(
    args: &RunArgs,
    exe: &Path,
    game_dir: &Path,
    allowed: &[PathBuf],
    prefix: &Path,
) -> RunArgs {
    let proton = args.proton.as_deref().map(|asked| match std::fs::canonicalize(asked) {
        Ok(folder) if folder.is_dir() => folder.to_string_lossy().into_owned(),
        _ => asked.to_string(),
    });
    RunArgs {
        exe: exe.to_path_buf(),
        game_dir: args.game_dir.as_ref().map(|_| game_dir.to_path_buf()),
        allow: allowed.to_vec(),
        prefix: args.prefix.as_ref().map(|_| crate::canonical(prefix)),
        proton,
        ..args.clone()
    }
}

/// The prefix another launcher made for the game at `exe`, for `--take-ownership` and
/// `--prefix-continue` alike: the one there is, or, where there are several, the one picked by its
/// number from a menu. Why not, where no launcher has one, the menu can't show, or nothing is
/// picked.
fn past_prefix(exe: &Path, home: &Path) -> Result<PastPrefix, String> {
    let found = prefix::made_elsewhere(exe, home);
    let several = match found.as_slice() {
        [] => {
            let launchers = prefix::launchers().join(", ");
            return Err(format!(
                "no launcher here has a prefix for {} (looked in: {launchers})",
                exe.display()
            ));
        }
        [one] => return Ok(one.clone()),
        several => several,
    };
    let intro = format!("More than one prefix was made for {}:", exe.display());
    let menu = terminal_choice::Menu::new("Which one?").intro(intro);
    let mut menu = several.iter().fold(menu, |menu, past| menu.pick(past.described()));
    match terminal_choice::run_menu(&mut menu) {
        Ok(terminal_choice::Chosen::Picked(at)) => Ok(several[at].clone()),
        Ok(terminal_choice::Chosen::Cancelled) => {
            Err("no prefix was picked, so nothing ran".to_string())
        }
        Err(err) => {
            let listed: Vec<String> = several.iter().map(PastPrefix::described).collect();
            Err(format!(
                "more than one prefix was made for {} — {} — and the menu to pick one can't show: {err}",
                exe.display(),
                listed.join("; ")
            ))
        }
    }
}

/// The line said as the game starts: its Proton, what it runs on, whether it has the network, and
/// where its own folder is; then whatever the pick needs said, and what the sandbox goes without
/// here — each such note a line of its own, signed with `name`.
fn summary(
    name: &str,
    choice: &proton::Choice,
    plan: &Plan,
    host: &Host,
    xwayland_asked: bool,
) -> String {
    let display = match (plan.wayland, &host.x11) {
        (true, _) => "Wayland",
        (false, Some(_)) => "X11",
        (false, None) => "no display",
    };
    let network = if plan.network { "on" } else { "off" };
    let mut line = format!(
        "{} on {display}, network {network}; the game's own folder: {}",
        choice.proton.name,
        plan.prefix.display()
    );
    let no_wayland = (!xwayland_asked && host.wayland.is_none())
        .then(|| "this session has no Wayland display".to_string());
    let unfiltered = host.filter_exe.is_none().then(|| {
        format!(
            "no syscall filter for {} yet: the game's calls go unchecked",
            std::env::consts::ARCH
        )
    });
    let userns = (!host.userns_locked).then(|| {
        "this bubblewrap can't stop the game making user namespaces (that takes 0.8 or later, not setuid)".to_string()
    });
    for note in choice.note.clone().into_iter().chain(no_wayland).chain(unfiltered).chain(userns) {
        line.push_str(&format!("\n{name}: {note}"));
    }
    line
}

/// The program the sandbox runs under for performance, where one is installed: CachyOS's
/// `game-performance` (the performance power profile while the game runs, and no idle sleep), or
/// Feral's GameMode.
fn performance_wrapper() -> Option<&'static str> {
    PERFORMANCE_WRAPPERS.iter().copied().find(|wrapper| sandbox::on_path(wrapper))
}

/// The performance wrappers, the first one installed taken.
const PERFORMANCE_WRAPPERS: &[&str] = &["game-performance", "gamemoderun"];

/// `PATH` without the folders `off_path` names, for the performance wrapper.
fn path_without(off_path: &[PathBuf]) -> OsString {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let kept = std::env::split_paths(&path).filter(|dir| !off_path.contains(dir));
    std::env::join_paths(kept).unwrap_or(path)
}

/// `word` as a shell reads it back: bare where it may be, else in single quotes — a quote inside it
/// written `'\''` — which bash, zsh and fish read alike, but for a backslash, which fish's quotes
/// take as an escape.
fn quoted(word: &OsStr) -> String {
    let text = word.to_string_lossy();
    let bare = !text.is_empty()
        && text.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:,+@%".contains(c));
    match bare {
        true => text.into_owned(),
        false => format!("'{}'", text.replace('\'', r"'\''")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The off-path folders leave `PATH` for the performance wrapper; the rest stays, in order.
    #[test]
    fn the_wrappers_path_leaves_out_the_programs_own_folders() {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let first = std::env::split_paths(&path).next();
        let off: Vec<PathBuf> = first.iter().cloned().collect();
        let kept: Vec<PathBuf> =
            std::env::split_paths(&path).filter(|dir| !off.contains(dir)).collect();
        assert_eq!(std::env::split_paths(&path_without(&off)).collect::<Vec<_>>(), kept);
        assert_eq!(path_without(&[]), path);
    }

    /// A word is bare where every shell reads it as it is, and single-quoted otherwise, a quote in
    /// it closing, escaping and reopening.
    #[test]
    fn words_are_quoted_where_a_shell_would_read_them_otherwise() {
        assert_eq!(quoted(OsStr::new("--ro-bind")), "--ro-bind");
        assert_eq!(quoted(OsStr::new("/home/u/Games/x-1a2b")), "/home/u/Games/x-1a2b");
        assert_eq!(quoted(OsStr::new("Some Game")), "'Some Game'");
        assert_eq!(quoted(OsStr::new("it's")), r"'it'\''s'");
        assert_eq!(quoted(OsStr::new("")), "''");
        assert_eq!(quoted(OsStr::new("$HOME")), "'$HOME'");
    }

    /// A run, as told to the embedding program, holds the paths it settled on: the .exe, the folders
    /// allowed, and the game's folder and its own where they were given (its own followed through a
    /// link), and a Proton's folder whole where one was named by it — a name stays a name. The rest
    /// is as given, --take-ownership too.
    #[test]
    fn a_run_is_told_with_the_paths_it_settled_on() {
        let base = crate::proton::tests::scratch("as_run");
        let (protons, prefixes) = (base.join("protons/stand-in"), base.join("prefixes/game"));
        for dir in [&protons, &prefixes] {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::os::unix::fs::symlink(base.join("prefixes"), base.join("linked")).unwrap();
        let (exe, game_dir) = (base.join("games/x/bin/x.exe"), base.join("games/x"));
        let allowed = [base.join("mods")];
        let given = RunArgs {
            exe: PathBuf::from("x.exe"),
            game_args: vec!["-windowed".into()],
            xwayland: true,
            network: false,
            proton: Some(protons.to_str().unwrap().to_string()),
            game_dir: Some(PathBuf::from("..")),
            allow: vec![PathBuf::from("mods")],
            prefix: Some(PathBuf::from("game")),
            take_ownership: true,
            prefix_continue: false,
            no_performance: true,
            dry_run: false,
        };
        let told = as_run(&given, &exe, &game_dir, &allowed, &base.join("linked/game"));
        let expected = RunArgs {
            exe: exe.clone(),
            game_dir: Some(game_dir.clone()),
            allow: allowed.to_vec(),
            prefix: Some(crate::canonical(&prefixes)),
            proton: Some(crate::canonical(&protons).to_str().unwrap().to_string()),
            ..given.clone()
        };
        assert_eq!(told, expected);
        assert!(told.take_ownership);

        let bare = RunArgs {
            proton: Some("cachyos".into()),
            game_dir: None,
            prefix: None,
            prefix_continue: true,
            take_ownership: false,
            ..given
        };
        let told = as_run(&bare, &exe, &game_dir, &allowed, &base.join("elsewhere"));
        assert_eq!(
            (told.proton.as_deref(), told.game_dir, told.prefix),
            (Some("cachyos"), None, None)
        );
        assert!(told.prefix_continue);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A run with every option it can take at once (each of a conflicting pair aside), and
    /// arguments for the game that look like options.
    fn everything() -> RunArgs {
        RunArgs {
            exe: "/games/Baldur's Gate 3/bin/bg3.exe".into(),
            game_args: vec!["--network".into(), "two words".into(), "--".into()],
            xwayland: true,
            network: true,
            proton: Some("GE-Proton11-12".into()),
            game_dir: Some("/games/Baldur's Gate 3".into()),
            allow: vec!["/mods/a".into(), "/mods/b b".into()],
            prefix: Some("/prefixes/bg3".into()),
            prefix_continue: false,
            take_ownership: true,
            no_performance: true,
            dry_run: true,
        }
    }

    /// A run's words take the .exe first, then its options in the order they are declared —
    /// `--help`'s — every option among them.
    #[test]
    fn a_runs_words_take_the_exe_first_then_the_options_as_declared() {
        use clap::CommandFactory;
        let declared: Vec<String> = Cli::command()
            .get_arguments()
            .filter_map(|arg| arg.get_long())
            .filter(|long| !["help", "version"].contains(long))
            .map(|long| format!("--{long}"))
            .collect();
        let all = RunArgs { prefix_continue: true, ..everything() };
        let words: Vec<String> =
            all.words().iter().map(|word| word.to_string_lossy().into_owned()).collect();
        assert_eq!(words[0], "/games/Baldur's Gate 3/bin/bg3.exe");
        let options = &words[1..words.iter().position(|word| word == "--").unwrap()];
        let mut flags: Vec<&String> =
            options.iter().filter(|word| word.starts_with("--")).collect();
        flags.dedup(); // --allow, given twice
        assert_eq!(flags, declared.iter().collect::<Vec<_>>());
        assert_eq!(words[words.len() - 4..], ["--", "--network", "two words", "--"]);
    }

    /// A run's words read back as the very run: with no option and with every one, with a game's
    /// arguments that look like options or are `--` (or the first doesn't, and a later one does),
    /// and with paths that hold spaces and quotes. A .exe whose path starts with `-` reads back as
    /// the same file, from `./`.
    #[test]
    fn a_runs_words_read_back_as_the_run() {
        let bare = |exe: &str| Cli::try_parse_from(["win_ter", exe]).unwrap().run;
        let continued = RunArgs {
            prefix: None,
            take_ownership: false,
            prefix_continue: true,
            game_args: vec!["-windowed".into()],
            ..everything()
        };
        let later = RunArgs {
            game_args: vec!["windowed".into(), "--network".into(), "-h".into()],
            ..bare("/games/ice/Game.exe")
        };
        for given in [bare("/games/ice/Game.exe"), everything(), continued, later] {
            let words = given.words();
            let parsed =
                Cli::try_parse_from(std::iter::once("win_ter".into()).chain(words.clone()))
                    .unwrap_or_else(|err| panic!("{words:?}: {err}"));
            assert_eq!(parsed.run, given, "{words:?}");
        }
        assert_eq!(bare("/games/ice/Game.exe").words(), ["/games/ice/Game.exe"]);

        let dashed = Cli::try_parse_from(["win_ter", "--", "-game.exe"]).unwrap().run;
        assert_eq!(dashed.exe, Path::new("-game.exe"));
        let again = Cli::try_parse_from(std::iter::once("win_ter".into()).chain(dashed.words()));
        assert_eq!(again.unwrap().run.exe, Path::new("./-game.exe"));
    }
}
