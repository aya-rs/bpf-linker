use std::{
    env,
    ffi::{OsStr, OsString},
    fs,
    os::unix::ffi::OsStrExt as _,
    path::PathBuf,
    process::Command,
};

use anyhow::{Context as _, Result, anyhow};
use reqwest::{
    blocking::Client,
    header::{ACCEPT, AUTHORIZATION, HeaderMap, USER_AGENT},
};
use rustc_build_sysroot::{BuildMode, SysrootConfig, SysrootStatus};
use serde::Deserialize;
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
    #[arg(long)]
    rustc_src: PathBuf,

    #[arg(long)]
    sysroot_dir: PathBuf,

    #[arg(long, value_enum)]
    target: Target,
}

#[derive(clap::Parser)]
struct BuildLlvm {
    /// Source directory.
    #[arg(long)]
    src_dir: PathBuf,
    /// Build directory.
    #[arg(long)]
    build_dir: PathBuf,
    /// Directory in which the built LLVM artifacts are installed.
    #[arg(long)]
    install_prefix: PathBuf,
}

#[derive(clap::Args)]
struct RustcLlvmCommitOptions {
    /// GitHub token used for API requests. Reads from `GITHUB_TOKEN` when unset.
    #[arg(long = "github-token", env = "GITHUB_TOKEN")]
    github_token: Option<String>,
}

#[derive(clap::Subcommand)]
enum XtaskSubcommand {
    /// Builds the Rust standard library for the given target in the current
    /// toolchain's sysroot.
    BuildStd(BuildStd),
    /// Manages and builds LLVM.
    BuildLlvm(BuildLlvm),
    /// Finds the commit in github.com/rust-lang/rust that can be used for
    /// downloading LLVM for the current Rust toolchain.
    RustcLlvmCommit(RustcLlvmCommitOptions),
}

/// Additional build commands for bpf-linker.
#[derive(clap::Parser)]
struct CommandLine {
    #[command(subcommand)]
    subcommand: XtaskSubcommand,
}

fn build_std(options: BuildStd) -> Result<()> {
    let BuildStd {
        rustc_src,
        sysroot_dir,
        target,
    } = options;

    let target = target.as_str();
    let sysroot_status =
        match rustc_build_sysroot::SysrootBuilder::new(sysroot_dir.as_path(), target)
            // Do a full sysroot build.
            .build_mode(BuildMode::Build)
            // We want only `core`, not `std`.
            .sysroot_config(SysrootConfig::NoStd)
            // Include debug symbols in order to generate correct BTF types for
            // the core types as well.
            .rustflag("-Cdebuginfo=2")
            .build_from_source(&rustc_src)?
        {
            SysrootStatus::AlreadyCached => "was already built",
            SysrootStatus::SysrootBuilt => "built successfully",
        };
    println!(
        "Standard library for target {target} {sysroot_status}: {}",
        sysroot_dir.display()
    );
    Ok(())
}

fn build_llvm(options: BuildLlvm) -> Result<()> {
    let BuildLlvm {
        src_dir,
        build_dir,
        install_prefix,
    } = options;

    let mut install_arg = OsString::from("-DCMAKE_INSTALL_PREFIX=");
    install_arg.push(install_prefix.as_os_str());
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
    let status = cmake_configure.status().with_context(|| {
        format!("failed to configure LLVM build with command {cmake_configure:?}")
    })?;
    if !status.success() {
        anyhow::bail!("failed to configure LLVM build with command {cmake_configure:?}: {status}");
    }

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
    let status = cmake_build
        .status()
        .with_context(|| format!("failed to build LLVM with command {cmake_configure:?}"))?;
    if !status.success() {
        anyhow::bail!("failed to configure LLVM build with command {cmake_configure:?}: {status}");
    }

    // Move targets over the symlinks that point to them.
    //
    // This whole dance would be simpler if CMake supported
    // `CMAKE_INSTALL_MODE=MOVE`.
    for entry in WalkDir::new(&install_prefix).follow_links(false) {
        let entry = entry.with_context(|| {
            format!(
                "failed to read filesystem entry while traversing install prefix {}",
                install_prefix.display()
            )
        })?;
        if !entry.file_type().is_symlink() {
            continue;
        }

        let link_path = entry.path();
        let target = fs::read_link(link_path)
            .with_context(|| format!("failed to read the link {}", link_path.display()))?;
        if target.is_absolute() {
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

#[derive(Deserialize)]
struct Commit {
    sha: String,
}

#[derive(Deserialize)]
struct PullRequest {
    merge_commit_sha: String,
}

fn expect_single<'a, T: std::fmt::Debug>(slice: &'a [T], what: &'a str) -> Result<&'a T> {
    match slice {
        [] => anyhow::bail!("found no instances of `{what}`"),
        [one] => Ok(one),
        slice => anyhow::bail!("found multiple instances of `{what}`: {slice:?}"),
    }
}

/// Finds a commit in the [Rust GitHub repository][rust-repo] that corresponds
/// to an update of LLVM and can be used to download libLLVM from Rust CI.
///
/// [rust-repo]: https://github.com/rust-lang/rust
fn rustc_llvm_commit(options: RustcLlvmCommitOptions) -> Result<()> {
    let RustcLlvmCommitOptions { github_token } = options;
    let toolchain = env::var_os("RUSTUP_TOOLCHAIN");

    let mut rustc_cmd = Command::new("rustc");
    if let Some(toolchain) = toolchain {
        let mut toolchain_arg = OsString::new();
        toolchain_arg.push(toolchain);
        let _: &mut Command = rustc_cmd.arg(toolchain_arg);
    }
    let output = rustc_cmd
        .args(["--version", "--verbose"])
        .output()
        .with_context(|| format!("failed to run {rustc_cmd:?}"))?;

    if !output.status.success() {
        return Err(anyhow!(
            "{rustc_cmd:?} failed with status {}",
            output.status
        ));
    }

    // `rustc --version --verbose` output should contain lines starting from:
    //
    // - `commit-hash:`
    // - `release:`
    // - `LLVM version:`
    //
    // Example:
    //
    // ```
    // commit-hash: 31010ca61c3ff019e1480dda0a7ef16bd2bd51c0
    // release: 1.94.0-nightly
    // LLVM version: 21.1.8
    // ```
    let mut commit_hashes = Vec::new();
    for line in output.stdout.split(|&b| b == b'\n') {
        if let Some(commit_hash) = line.strip_prefix(b"commit-hash: ") {
            commit_hashes.push(commit_hash);
        }
    }

    // For stable Rust, CI does not publish LLVM tarballs per commit. Instead,
    // we locate the most recent ancestor of the current commit that touched
    // `src/llvm-project` then resolve it back to its PR merge commit.

    let commit_hash = expect_single(&commit_hashes, "commit-hash:").with_context(|| {
        format!(
            "{rustc_cmd:?}: {}",
            OsStr::from_bytes(&output.stdout).display()
        )
    })?;

    // reqwest does not accept raw bytes.
    let commit_hash = str::from_utf8(commit_hash).with_context(|| {
        format!(
            "rust commit hash is not valid UTF-8: {}",
            OsStr::from_bytes(commit_hash).display()
        )
    })?;

    let mut headers: HeaderMap = [
        // GitHub requires a User-Agent header; requests without one get a 403.
        // Any non-empty value works, but we provide an identifier for this tool.
        (USER_AGENT, "bpf-linker-xtask/0.1".parse().unwrap()),
        (ACCEPT, "application/vnd.github+json".parse().unwrap()),
    ]
    .into_iter()
    .collect();
    if let Some(github_token) = github_token {
        assert_eq!(
            headers.insert(
                AUTHORIZATION,
                format!("Bearer {github_token}").parse().unwrap(),
            ),
            None
        );
    }
    let client = Client::builder()
        .default_headers(headers)
        .build()
        .with_context(|| "failed to build an HTTP client")?;

    const COMMITS_URL: &str = "https://api.github.com/repos/rust-lang/rust/commits";
    let resp = client
        .get(COMMITS_URL)
        .query(&[
            ("path", "src/llvm-project"),
            ("sha", commit_hash),
            // We only need the most recent match.
            ("per_page", "1"),
        ])
        .send()
        .with_context(|| format!("failed to send the request to {COMMITS_URL}"))?
        .error_for_status()
        .with_context(|| format!("HTTP request to {COMMITS_URL} returned an error status"))?;
    let [Commit { sha }] = resp.json()?;

    let pulls_url = format!("https://api.github.com/repos/rust-lang/rust/commits/{sha}/pulls");
    let resp = client
        .get(&pulls_url)
        .query(&[("per_page", "1")])
        .send()
        .with_context(|| format!("failed to send the request to {pulls_url}"))?
        .error_for_status()
        .with_context(|| format!("HTTP request to {pulls_url} returned an error status"))?;
    let [PullRequest { merge_commit_sha }] = resp.json()?;

    println!("{merge_commit_sha}");

    Ok(())
}

fn main() -> Result<()> {
    let CommandLine { subcommand } = clap::Parser::parse();
    match subcommand {
        XtaskSubcommand::BuildStd(options) => build_std(options),
        XtaskSubcommand::BuildLlvm(options) => build_llvm(options),
        XtaskSubcommand::RustcLlvmCommit(options) => rustc_llvm_commit(options),
    }
}
