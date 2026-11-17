//! Docker (Docker Desktop, or the Docker Engine on Linux) with the compose plugin.

use super::{Ctx, Runtime, RuntimeInfo};
use anyhow::{Result, bail};

pub struct Docker<'a> {
    ctx: Ctx<'a>,
}

impl<'a> Docker<'a> {
    pub fn new(ctx: Ctx<'a>) -> Self {
        Self { ctx }
    }
}

impl Runtime for Docker<'_> {
    fn ctx(&self) -> &Ctx<'_> {
        &self.ctx
    }

    fn name(&self) -> &'static str {
        "Docker"
    }

    fn program(&self) -> &'static str {
        "docker"
    }

    fn captured_flags(&self) -> &'static [&'static str] {
        &["--ansi", "never"]
    }

    fn not_running_message(&self) -> String {
        "Docker is not running. Start Docker Desktop (or the Docker service) and try again.".into()
    }

    fn check_running(&self) -> Result<RuntimeInfo> {
        let r = self.run(&["info", "--format", "{{.ServerVersion}}"]);
        if r.exit_code != 0 {
            bail!(self.not_running_message());
        }
        Ok(RuntimeInfo {
            version: r.stdout.trim().to_string(),
        })
    }
}
