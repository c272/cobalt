use std::{env, sync::Arc};

use colored::Colorize;
use cranelift::codegen::{
    isa::TargetIsa,
    settings::{self, Configurable},
};
use miette::Result;

use crate::config::BuildConfig;

/// An abstract representation of an ISA within the Cobalt code generator.
pub(super) enum Isa {
    #[allow(non_camel_case_types)]
    x86_64,
    Aarch64,
}

impl Isa {
    /// Creates a new ISA structure based on the current hardware platform.
    pub(super) fn new_from_platform() -> Result<Self> {
        match env::consts::ARCH {
            "x86_64" => Ok(Self::x86_64),
            "aarch64" => Ok(Self::Aarch64),
            arch => {
                miette::bail!(
                    "Incompatible architecture '{}' detected for code generation.",
                    arch
                );
            }
        }
    }

    /// Converts the given ISA into a Cranelift ISA structure.
    pub(super) fn to_cranelift_isa(self, cfg: &BuildConfig) -> Result<Arc<dyn TargetIsa>> {
        // Create flag builder, insert default settings.
        let mut flag_builder = settings::builder();
        let mut isa_builder = cranelift_native::builder().unwrap();
        flag_builder.set("use_colocated_libcalls", "false").unwrap();
        flag_builder.set("is_pic", "false").unwrap();

        // Set optimisation level.
        flag_builder.set("opt_level", &cfg.opt_level).unwrap();

        // Add any platform-specific settings.
        match self {
            Self::x86_64 => {}
            Self::Aarch64 => {
                // Only enable if security features are enabled.
                if cfg.gen_security_features {
                    // Enable PAC-RET for all functions.
                    isa_builder.enable("sign_return_address").unwrap();
                    isa_builder.enable("sign_return_address_all").unwrap();

                    // Enable BTI.
                    isa_builder.enable("use_bti").unwrap();
                } else {
                    println!("{}", "warn [🚧]: Hardware security instruction generation disabled for generated binaries.".yellow());
                }
            }
        }

        // Build the ISA.
        isa_builder
            .finish(settings::Flags::new(flag_builder))
            .map_err(|err| miette::diagnostic!("codegen: Failed to create ISA: {}", err).into())
    }
}
