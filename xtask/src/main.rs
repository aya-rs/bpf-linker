use std::{
    env,
    ffi::OsString,
    fs,
    io::{BufRead as _, BufReader, Read as _},
    os::unix::ffi::OsStringExt as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::Duration,
};

use anyhow::{Context as _, Result, bail};
use flate2::read::GzDecoder;
use indicatif::{ProgressBar, ProgressStyle};
use rustc_build_sysroot::{BuildMode, SysrootConfig, SysrootStatus};
use tar::Archive;
use walkdir::WalkDir;

#[derive(Clone, clap::ValueEnum)]
enum Target {
    BpfebUnknownNone,
    BpfelUnknownNone,
}

impl Target {
    fn as_str(&self) -> &'static str {
        match self {
            Self::BpfebUnknownNone => "bpfeb-unknown-none",
            Self::BpfelUnknownNone => "bpfel-unknown-none",
        }
    }
}

#[derive(clap::Parser)]
struct BuildStd {
    #[arg(long, value_enum, default_value_t = Target::BpfelUnknownNone)]
    target: Target,
}

#[derive(clap::Parser)]
struct BuildLlvm {
    /// Source directory. If not provided, LLVM sources are downloaded from
    /// GitHub.
    #[arg(long)]
    src_dir: Option<PathBuf>,
    /// Directory in which the built LLVM artifacts are installed.
    #[arg(long)]
    install_dir: PathBuf,
}

#[derive(clap::Subcommand)]
enum XtaskSubcommand {
    /// Builds the Rust standard library for the given target in the current
    /// toolchain's sysroot.
    BuildStd(BuildStd),
    /// Builds LLVM.
    BuildLlvm(BuildLlvm),
}

/// Additional build commands for bpf-linker.
#[derive(clap::Parser)]
struct CommandLine {
    #[command(subcommand)]
    subcommand: XtaskSubcommand,
}

fn sysroot_dir() -> Result<PathBuf> {
    let mut rustc = Command::new(env::var_os("RUSTC").unwrap_or_else(|| OsString::from("rustc")));
    let output = rustc
        .args(["--print", "sysroot"])
        .output()
        .with_context(|| format!("failed to derermine rustc sysroot: {rustc:?}"))?;
    if !output.status.success() {
        bail!("failed to determine rustc sysroot: {output:?}");
    }
    Ok(PathBuf::from(OsString::from_vec(
        output.stdout.trim_ascii().to_vec(),
    )))
}

fn build_std(options: BuildStd) -> Result<()> {
    let BuildStd { target } = options;

    let sysroot_dir = sysroot_dir()?;
    let source_dir = sysroot_dir
        .join("lib")
        .join("rustlib")
        .join("src")
        .join("rust")
        .join("library");
    let target = target.as_str();
    let rustlibdir = sysroot_dir.join("lib").join("rustlib").join(target);
    let sysroot_status =
        match rustc_build_sysroot::SysrootBuilder::new(sysroot_dir.as_path(), target)
            // Do a full sysroot build.
            .build_mode(BuildMode::Build)
            // We want only `core`, not `std`.
            .sysroot_config(SysrootConfig::NoStd)
            // Include debug symbols in order to generate correct BTF types for
            // the core types as well.
            .rustflag("-Cdebuginfo=2")
            .build_from_source(&source_dir)?
        {
            SysrootStatus::AlreadyCached => "was already built",
            SysrootStatus::SysrootBuilt => "built successfully",
        };
    println!(
        "Standard library for target {target} {sysroot_status}: {}",
        rustlibdir.display()
    );
    Ok(())
}

fn progress_bar_spinner(template: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .template(template)
            .expect("failed to create a progress bar with template")
            .tick_strings(&["⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷"]),
    );
    pb.enable_steady_tick(Duration::from_millis(100));
    pb
}

fn download_llvm_sources() -> Result<Vec<u8>> {
    /// Branch of [Aya's LLVM fork][aya-llvm] to be used. It should correspond
    /// to the LLVM version currently used in Rust nightly.
    const BRANCH: &str = "rustc/21.1-2025-08-01";

    let client = reqwest::blocking::Client::new();

    let url = format!("https://github.com/aya-rs/llvm-project/archive/refs/heads/{BRANCH}.tar.gz");
    let mut response = client
        .get(&url)
        .send()
        .with_context(|| format!("failed to sent a GET request to {url}"))?;

    if !response.status().is_success() {
        anyhow::bail!("HTTP error: {}", response.status());
    }

    let pb = progress_bar_spinner("{spinner:.green} [{elapsed_precise}] {bytes} downloaded...");
    let mut downloaded: u64 = 0;
    let mut buf = vec![0; 8192];
    let mut data = Vec::new();
    loop {
        let bytes_read = response
            .read(&mut buf)
            .with_context(|| format!("failed to read response to a request to {url}"))?;
        if bytes_read == 0 {
            break;
        }
        data.extend_from_slice(&buf[..bytes_read]);
        downloaded += bytes_read as u64;
        pb.set_position(downloaded);
    }
    pb.finish_with_message("Download completed");

    Ok(data)
}

fn unpack_llvm_sources(build_dir: &Path, data: Vec<u8>) -> Result<PathBuf> {
    let gz_decoder = GzDecoder::new(&data[..]);
    let mut archive = Archive::new(gz_decoder);

    let pb = progress_bar_spinner("{spinner:.green} [{elapsed_precise}] unpacking...");
    archive.unpack(build_dir).with_context(|| {
        format!(
            "failed to unpack the LLVM source archive into {}",
            build_dir.display()
        )
    })?;
    pb.finish_with_message("Unpacking completed");

    let subdir = fs::read_dir(build_dir)
        .with_context(|| format!("failed to read the build directory {}", build_dir.display()))?
        .filter_map(|res| res.ok())
        .map(|dir| dir.path())
        .next()
        .unwrap();

    Ok(subdir)
}

fn execute_command(cmd: &mut Command) -> Result<()> {
    let cmd = cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .with_context(|| format!("failed to spawn command {cmd:?}"))?;
    let stdout = child
        .stdout
        .take()
        .unwrap_or_else(|| panic!("expected piped stdout in command {cmd:?}"));
    let stderr = child
        .stderr
        .take()
        .unwrap_or_else(|| panic!("expected piped stderr in command {cmd:?}"));
    thread::scope(|s| {
        let _stdout_handle = s.spawn(|| {
            for line in BufReader::new(stdout).lines() {
                let line = line.unwrap_or_else(|e| {
                    panic!("failed to retrieve stdout line from command {cmd:?}: {e:?}")
                });
                println!("{line}");
            }
        });
        let _stderr_handle = s.spawn(|| {
            for line in BufReader::new(stderr).lines() {
                let line = line.unwrap_or_else(|e| {
                    panic!("failed to retrieve stderr line from command {cmd:?}: {e:?}")
                });
                eprintln!("{line}")
            }
        });
    });

    Ok(())
}

/// Moves targets over the symlinks that point to them.
///
/// This whole dance would be simpler if CMake supported
/// `CMAKE_INSTALL_MODE=MOVE`.
fn move_absolute_symlink_targets(install_dir: &Path) -> Result<()> {
    for entry in WalkDir::new(install_dir)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_symlink())
    {
        let link_path = entry.path();
        let target = fs::read_link(link_path)
            .with_context(|| format!("failed to read the link {}", link_path.display()))?;
        if target.is_absolute() {
            // Move the file the link points to into the location of the symlink
            fs::rename(&target, link_path).with_context(|| {
                format!(
                    "failed to move the target file {} to the location of the symlink {}",
                    target.display(),
                    link_path.display()
                )
            })?;
        }
    }

    Ok(())
}

fn build_llvm(options: BuildLlvm) -> Result<()> {
    let BuildLlvm {
        src_dir,
        install_dir,
    } = options;

    let src_dir = match src_dir {
        Some(src_dir) => src_dir,
        None => {
            let src_dir = PathBuf::from("/tmp/bpf-linker-llvm");
            let llvm_sources = download_llvm_sources()?;
            unpack_llvm_sources(&src_dir, llvm_sources)?
        }
    };

    let build_dir = src_dir.join("build");

    let mut install_arg = OsString::from("-DCMAKE_INSTALL_PREFIX=");
    install_arg.push(install_dir.as_os_str());
    let mut cmake_configure = Command::new("cmake");
    let cmake_configure = cmake_configure
        .arg("-S")
        .arg(src_dir.join("llvm"))
        .arg("-B")
        .arg(&build_dir)
        .args([
            "-G",
            "Ninja",
            "-DCMAKE_BUILD_TYPE=RelWithDebInfo",
            "-DCMAKE_C_COMPILER=clang",
            "-DCMAKE_CXX_COMPILER=clang++",
            "-DLLVM_BUILD_LLVM_DYLIB=ON",
            "-DLLVM_ENABLE_ASSERTIONS=ON",
            "-DLLVM_ENABLE_PROJECTS=",
            "-DLLVM_ENABLE_RUNTIMES=",
            "-DLLVM_INSTALL_UTILS=ON",
            "-DLLVM_LINK_LLVM_DYLIB=ON",
            "-DLLVM_TARGETS_TO_BUILD=BPF",
            "-DLLVM_USE_LINKER=lld",
        ])
        .arg(install_arg);
    println!("Configuring LLVM with command {cmake_configure:?}");
    execute_command(cmake_configure)?;

    let mut cmake_build = Command::new("cmake");
    let cmake_build = cmake_build
        .arg("--build")
        .arg(build_dir)
        .args(["--target", "install"])
        // Create symlinks rather than copies to conserve disk space,
        // especially on GitHub-hosted runners.
        //
        // Since the LLVM build creates a bunch of symlinks (and this setting
        // does not turn those into symlinks-to-symlinks), use absolute
        // symlinks so we can distinguish the two cases.
        .env("CMAKE_INSTALL_MODE", "ABS_SYMLINK");
    println!("Building LLVM with command {cmake_build:?}");
    execute_command(cmake_build)?;

    move_absolute_symlink_targets(&install_dir)?;

    Ok(())
}

fn main() -> Result<()> {
    let CommandLine { subcommand } = clap::Parser::parse();
    match subcommand {
        XtaskSubcommand::BuildStd(options) => build_std(options),
        XtaskSubcommand::BuildLlvm(options) => build_llvm(options),
    }
}
