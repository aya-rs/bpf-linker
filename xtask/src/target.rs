use std::ffi::{OsStr, OsString};

use clap::ValueEnum;
use target_lexicon::{
    Aarch64Architecture, Architecture, BinaryFormat, Environment, OperatingSystem,
    Riscv64Architecture, Triple, Vendor,
};

use crate::llvm::{LlvmBuildConfig, Processor, System};

#[derive(Clone)]
pub enum SupportedTriple {
    Aarch64AppleDarwin,
    Aarch64UnknownLinuxGnu,
    Aarch64UnknownLinuxMusl,
    Riscv64GcUnknownLinuxGnu,
    X86_64AppleDarwin,
    X86_64UnknownLinuxGnu,
    X86_64UnknownLinuxMusl,
}

impl ValueEnum for SupportedTriple {
    fn value_variants<'a>() -> &'a [Self] {
        &[
            Self::Aarch64AppleDarwin,
            Self::Aarch64UnknownLinuxGnu,
            Self::Aarch64UnknownLinuxMusl,
            Self::Riscv64GcUnknownLinuxGnu,
            Self::X86_64AppleDarwin,
            Self::X86_64UnknownLinuxGnu,
            Self::X86_64UnknownLinuxMusl,
        ]
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        Some(match self {
            Self::Aarch64AppleDarwin => clap::builder::PossibleValue::new("aarch64-apple-darwin"),
            Self::Aarch64UnknownLinuxGnu => {
                clap::builder::PossibleValue::new("aarch64-unknown-linux-gnu")
            }
            Self::Aarch64UnknownLinuxMusl => {
                clap::builder::PossibleValue::new("aarch64-unknown-linux-musl")
            }
            Self::Riscv64GcUnknownLinuxGnu => {
                clap::builder::PossibleValue::new("riscv64gc-unknown-linux-gnu")
            }
            Self::X86_64AppleDarwin => clap::builder::PossibleValue::new("x86_64-apple-darwin"),
            Self::X86_64UnknownLinuxGnu => {
                clap::builder::PossibleValue::new("x86_64-unknown-linux-gnu")
            }
            Self::X86_64UnknownLinuxMusl => {
                clap::builder::PossibleValue::new("x86_64-unknown-linux-musl")
            }
        })
    }
}

impl From<SupportedTriple> for Triple {
    fn from(value: SupportedTriple) -> Self {
        match value {
            SupportedTriple::Aarch64AppleDarwin => Triple {
                architecture: Architecture::Aarch64(Aarch64Architecture::Aarch64),
                vendor: Vendor::Apple,
                operating_system: OperatingSystem::Darwin,
                environment: Environment::Unknown,
                binary_format: BinaryFormat::Macho,
            },
            SupportedTriple::Aarch64UnknownLinuxGnu => Triple {
                architecture: Architecture::Aarch64(Aarch64Architecture::Aarch64),
                vendor: Vendor::Unknown,
                operating_system: OperatingSystem::Linux,
                environment: Environment::Gnu,
                binary_format: BinaryFormat::Elf,
            },
            SupportedTriple::Aarch64UnknownLinuxMusl => Triple {
                architecture: Architecture::Aarch64(Aarch64Architecture::Aarch64),
                vendor: Vendor::Unknown,
                operating_system: OperatingSystem::Linux,
                environment: Environment::Musl,
                binary_format: BinaryFormat::Elf,
            },
            SupportedTriple::Riscv64GcUnknownLinuxGnu => Triple {
                architecture: Architecture::Riscv64(Riscv64Architecture::Riscv64gc),
                vendor: Vendor::Unknown,
                operating_system: OperatingSystem::Linux,
                environment: Environment::Gnu,
                binary_format: BinaryFormat::Elf,
            },
            SupportedTriple::X86_64AppleDarwin => Triple {
                architecture: Architecture::X86_64,
                vendor: Vendor::Apple,
                operating_system: OperatingSystem::Darwin,
                environment: Environment::Unknown,
                binary_format: BinaryFormat::Macho,
            },
            SupportedTriple::X86_64UnknownLinuxGnu => Triple {
                architecture: Architecture::X86_64,
                vendor: Vendor::Unknown,
                operating_system: OperatingSystem::Linux,
                environment: Environment::Gnu,
                binary_format: BinaryFormat::Elf,
            },
            SupportedTriple::X86_64UnknownLinuxMusl => Triple {
                architecture: Architecture::X86_64,
                vendor: Vendor::Unknown,
                operating_system: OperatingSystem::Linux,
                environment: Environment::Musl,
                binary_format: BinaryFormat::Elf,
            },
        }
    }
}

pub trait TripleExt {
    fn containerized_build(&self) -> bool;
    fn container_image(
        &self,
        container_repository: &str,
        container_tag: &str,
    ) -> Option<(String, String)>;
    fn llvm_build_config(&self, install_prefix: &OsStr) -> Option<LlvmBuildConfig>;
    fn libdirs(&self) -> Vec<OsString>;
    fn sysroot(&self) -> Option<OsString>;
    fn is_cross(&self) -> bool;
}

impl TripleExt for Triple {
    fn containerized_build(&self) -> bool {
        let Triple {
            operating_system, ..
        } = self;
        *operating_system == OperatingSystem::Linux
    }

    fn container_image(
        &self,
        container_repository: &str,
        container_tag: &str,
    ) -> Option<(String, String)> {
        let prefix = if self.is_cross() { "cross" } else { "native" };
        if self.containerized_build() {
            let image_name = format!("{prefix}-{self}:{container_tag}");
            let full_name = format!("{container_repository}/{image_name}");
            let dockerfile = format!("docker/Dockerfile.{image_name}");
            Some((full_name, dockerfile))
        } else {
            None
        }
    }

    fn llvm_build_config(&self, install_prefix: &OsStr) -> Option<LlvmBuildConfig> {
        let Triple {
            architecture,
            operating_system,
            environment,
            ..
        } = self;
        let install_prefix = install_prefix.to_owned();

        match (architecture, operating_system, environment) {
            (Architecture::Aarch64(_), OperatingSystem::Darwin, Environment::Unknown) => {
                Some(LlvmBuildConfig {
                    c_compiler: "clang".to_owned(),
                    cxx_compiler: "clang++".to_owned(),
                    compiler_target: None,
                    cxxflags: None,
                    ldflags: None,
                    install_prefix,
                    skip_install_rpath: false,
                    system: System::Darwin,
                    processor: Processor::Aarch64,
                    static_build: false,
                    target_triple: "aarch64-apple-darwin".to_owned(),
                })
            }
            (Architecture::Aarch64(_), OperatingSystem::Linux, Environment::Gnu) => {
                Some(LlvmBuildConfig {
                    c_compiler: "clang".to_owned(),
                    cxx_compiler: "clang++".to_owned(),
                    compiler_target: Some("aarch64-linux-gnu".to_owned()),
                    cxxflags: None,
                    ldflags: None,
                    install_prefix,
                    skip_install_rpath: false,
                    system: System::Linux,
                    processor: Processor::Aarch64,
                    static_build: true,
                    target_triple: "aarch64-linux-gnu".to_owned(),
                })
            }
            (Architecture::Aarch64(_), OperatingSystem::Linux, Environment::Musl) => {
                Some(LlvmBuildConfig {
                    c_compiler: if self.is_cross() {
                        "aarch64-unknown-linux-musl-clang".to_owned()
                    } else {
                        "clang".to_owned()
                    },
                    cxx_compiler: if self.is_cross() {
                        "aarch64-unknown-linux-musl-clang++".to_owned()
                    } else {
                        "clang++".to_owned()
                    },
                    // The clang wrapper specified above takes care of setting
                    // the target.
                    compiler_target: None,
                    cxxflags: Some("-stdlib=libc++".to_owned()),
                    ldflags: Some(
                        "-rtlib=compiler-rt -unwindlib=libunwind -lc++ -lc++abi".to_owned(),
                    ),
                    install_prefix,
                    skip_install_rpath: false,
                    system: System::Linux,
                    processor: Processor::Aarch64,
                    static_build: true,
                    target_triple: "aarch64-unknown-linux-musl".to_owned(),
                })
            }
            (Architecture::Riscv64(_), OperatingSystem::Linux, Environment::Gnu) => {
                Some(LlvmBuildConfig {
                    c_compiler: "clang".to_owned(),
                    cxx_compiler: "clang++".to_owned(),
                    compiler_target: Some("riscv64-linux-gnu".to_owned()),
                    cxxflags: None,
                    ldflags: None,
                    install_prefix,
                    skip_install_rpath: false,
                    system: System::Linux,
                    processor: Processor::Riscv64,
                    static_build: true,
                    target_triple: "riscv64-linux-gnu".to_owned(),
                })
            }
            (Architecture::X86_64, OperatingSystem::Darwin, Environment::Unknown) => {
                Some(LlvmBuildConfig {
                    c_compiler: "clang".to_owned(),
                    cxx_compiler: "clang++".to_owned(),
                    cxxflags: None,
                    compiler_target: None,
                    ldflags: None,
                    install_prefix,
                    skip_install_rpath: false,
                    system: System::Darwin,
                    processor: Processor::X86_64,
                    static_build: false,
                    target_triple: "x86_64-apple-darwin".to_owned(),
                })
            }
            (Architecture::X86_64, OperatingSystem::Linux, Environment::Gnu) => {
                Some(LlvmBuildConfig {
                    c_compiler: "clang".to_owned(),
                    cxx_compiler: "clang++".to_owned(),
                    compiler_target: Some("x86_64-linux-gnu".to_owned()),
                    cxxflags: None,
                    ldflags: None,
                    install_prefix,
                    skip_install_rpath: false,
                    system: System::Linux,
                    processor: Processor::X86_64,
                    static_build: true,
                    target_triple: "x86_64-linux-gnu".to_owned(),
                })
            }
            (Architecture::X86_64, OperatingSystem::Linux, Environment::Musl) => {
                Some(LlvmBuildConfig {
                    c_compiler: if self.is_cross() {
                        "x86_64-unknown-linux-musl-clang".to_owned()
                    } else {
                        "clang".to_owned()
                    },
                    cxx_compiler: if self.is_cross() {
                        "x86_64-unknown-linux-musl-clang++".to_owned()
                    } else {
                        "clang++".to_owned()
                    },
                    // The clang wrapper specified above takes care of setting
                    // the target.
                    compiler_target: None,
                    cxxflags: None,
                    ldflags: None,
                    install_prefix,
                    skip_install_rpath: false,
                    system: System::Linux,
                    processor: Processor::X86_64,
                    static_build: true,
                    target_triple: "x86_64-unknown-linux-musl".to_owned(),
                })
            }
            (_, _, _) => None,
        }
    }

    fn libdirs(&self) -> Vec<OsString> {
        if !self.is_cross() {
            return vec![OsString::from("/lib"), OsString::from("/usr/lib")];
        }

        let Triple {
            architecture,
            operating_system,
            environment,
            ..
        } = self;

        // Cross packages in Debian (which we use for GNU builds) install
        // libraries either in:
        //
        // - `/usr/<triple>/lib`
        // - `/usr/lib/<triple>`
        //
        // Cross packages in Gentoo (which we use for musl builds) are always
        // installed in the sysroot (`/usr/<triple>`), which follows the same
        // directory structure as host environments.
        match (architecture, operating_system, environment) {
            (Architecture::Aarch64(_), OperatingSystem::Linux, Environment::Gnu) => {
                vec![
                    OsString::from("/usr/aarch64-linux-gnu/lib"),
                    OsString::from("/usr/lib/aarch64-linux-gnu"),
                ]
            }
            (Architecture::Aarch64(_), OperatingSystem::Linux, Environment::Musl) => {
                vec![
                    OsString::from("/usr/aarch64-unknown-linux-musl/lib"),
                    OsString::from("/usr/aarch64-unknown-linux-musl/usr/lib"),
                ]
            }
            (Architecture::Riscv64(_), OperatingSystem::Linux, Environment::Gnu) => {
                vec![
                    OsString::from("/usr/riscv64-linux-gnu/lib"),
                    OsString::from("/usr/lib/riscv64-linux-gnu"),
                ]
            }
            (Architecture::Riscv64(_), OperatingSystem::Linux, Environment::Musl) => {
                vec![
                    OsString::from("/usr/riscv64-unknown-linux-musl/lib"),
                    OsString::from("/usr/riscv64-unknown-linux-musl/usr/lib"),
                ]
            }
            (Architecture::X86_64, OperatingSystem::Linux, Environment::Gnu) => {
                vec![
                    OsString::from("/usr/x86_64-linux-gnu/lib"),
                    OsString::from("/usr/lib/x86_64-linux-gnu"),
                ]
            }
            (Architecture::X86_64, OperatingSystem::Linux, Environment::Musl) => {
                vec![
                    OsString::from("/usr/x86_64-unknown-linux-musl/lib"),
                    OsString::from("/usr/x86_64-unknown-linux-musl/usr/lib"),
                ]
            }
            (_, _, _) => vec![OsString::from("/lib"), OsString::from("/usr/lib")],
        }
    }

    fn sysroot(&self) -> Option<OsString> {
        if !self.is_cross() {
            return None;
        }

        let Triple {
            architecture,
            operating_system,
            environment,
            ..
        } = self;

        match (architecture, operating_system, environment) {
            (Architecture::Aarch64(_), OperatingSystem::Linux, Environment::Musl) => {
                Some(OsString::from("/usr/aarch64-unknown-linux-musl"))
            }
            (Architecture::Riscv64(_), OperatingSystem::Linux, Environment::Musl) => {
                Some(OsString::from("/usr/riscv64-unknown-linux-musl"))
            }
            (Architecture::X86_64, OperatingSystem::Linux, Environment::Musl) => {
                Some(OsString::from("/usr/x86_64-unknown-linux-musl"))
            }
            (_, _, _) => None,
        }
    }

    fn is_cross(&self) -> bool {
        self.architecture != target_lexicon::HOST.architecture
    }
}
