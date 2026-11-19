//! Docker (Docker Desktop, or the Docker Engine on Linux) with the compose plugin.

use super::{Ctx, Runtime, RuntimeInfo, RuntimeKind};
use crate::ui::Ui;
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

    fn kind(&self) -> RuntimeKind {
        RuntimeKind::Docker
    }

    fn captured_flags(&self) -> &'static [&'static str] {
        &["--ansi", "never"]
    }

    fn not_running_message(&self) -> String {
        "Docker is not running. Start Docker Desktop (or the Docker service) and try again.".into()
    }

    fn check_running(&self, ui: &Ui) -> Result<RuntimeInfo> {
        let r = self.run(ui, &["info", "--format", "{{.ServerVersion}}"]);
        if r.exit_code != 0 {
            bail!(self.not_running_message());
        }
        Ok(RuntimeInfo {
            kind: self.kind(),
            version: r.stdout.trim().to_string(),
            rootless: false,
        })
    }

    fn gpu_hint(&self) -> &'static str {
        "Linux: install the NVIDIA Container Toolkit (AMD: the ROCm driver). \
         Windows: use Docker Desktop with the WSL2 backend and a current NVIDIA driver for Windows."
    }
}
