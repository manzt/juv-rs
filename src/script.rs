use std::{path::Path, str::FromStr};

#[derive(Debug, PartialEq)]
enum RuntimeKind {
    Notebook,
    Lab,
    Nbclassic,
}

#[derive(Debug, PartialEq)]
pub struct Runtime {
    kind: RuntimeKind,
    version: Option<String>,
}

impl FromStr for Runtime {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (kind_str, version) = if s.contains('@') {
            s.split_once('@')
                .map(|(name, version)| (name, Some(version.to_string())))
                .unwrap_or((s, None))
        } else if s.contains("==") {
            s.split_once("==")
                .map(|(name, version)| (name, Some(version.to_string())))
                .unwrap_or((s, None))
        } else {
            (s, None)
        };

        let kind = match kind_str {
            "notebook" => RuntimeKind::Notebook,
            "lab" => RuntimeKind::Lab,
            "nbclassic" => RuntimeKind::Nbclassic,
            _ => anyhow::bail!("Invalid runtime specifier: {}", s),
        };

        Ok(Runtime { kind, version })
    }
}

impl Runtime {
    fn exacutable(&self) -> &str {
        match self.kind {
            RuntimeKind::Notebook => "jupyter-notebook",
            RuntimeKind::Lab => "jupyter-lab",
            RuntimeKind::Nbclassic => "jupyter-nbclassic",
        }
    }
    fn main_import(&self) -> &str {
        if self.kind == RuntimeKind::Notebook && self.version.as_deref() == Some("6") {
            return "from notebook.notebookapp import main";
        };
        match self.kind {
            RuntimeKind::Notebook => "from notebook.app import main",
            RuntimeKind::Lab => "from jupyterlab.labapp import main",
            RuntimeKind::Nbclassic => "from nbclassic.notebookapp import main",
        }
    }
    fn name(&self) -> &str {
        match self.kind {
            RuntimeKind::Notebook => "notebook",
            RuntimeKind::Lab => "jupyterlab",
            RuntimeKind::Nbclassic => "nbclassic",
        }
    }
    pub fn run_script(
        &self,
        path: &Path,
        meta: Option<&str>,
        is_managed: bool,
        jupyter_args: &[String],
    ) -> String {
        let notebook = path.to_string_lossy().into_owned();
        let mut args = vec![self.exacutable().to_string(), notebook];
        args.extend_from_slice(jupyter_args);

        format!(
            r#"{meta}
import os
import sys

{runtime_main_import}

{SETUP_JUPYTER_DATA_DIR}

if {is_managed}:
    import importlib.metadata

    version = importlib.metadata.version("{runtime_name}")
    print("JUV_MANGED=" + "{runtime_name}" + "," + version, file=sys.stderr)

sys.argv = {sys_argv}
main()
"#,
            meta = meta.unwrap_or(""),
            runtime_main_import = self.main_import(),
            runtime_name = self.name(),
            is_managed = if is_managed { "True" } else { "False" },
            SETUP_JUPYTER_DATA_DIR = SETUP_JUPYTER_DATA_DIR,
            sys_argv = format!("{:?}", args)
        )
    }

    pub fn as_dependency_specifier(&self) -> String {
        let name = match self {
            Runtime {
                kind: RuntimeKind::Notebook,
                ..
            } => "notebook",
            Runtime {
                kind: RuntimeKind::Lab,
                ..
            } => "jupyterlab",
            Runtime {
                kind: RuntimeKind::Nbclassic,
                ..
            } => "nbclassic",
        };
        let specifier = if let Some(version) = &self.version {
            format!("{}=={}", name, version)
        } else {
            name.to_string()
        };
        if self.kind == RuntimeKind::Notebook && self.version.as_deref() == Some("6") {
            // notebook v6 requires setuptools
            format!("{},setuptools", specifier)
        } else {
            specifier
        }
    }
}

const SETUP_JUPYTER_DATA_DIR: &str = r#"
import tempfile
import signal
from pathlib import Path
import os
import sys

from platformdirs import user_data_dir

juv_data_dir = Path(user_data_dir("juv"))
juv_data_dir.mkdir(parents=True, exist_ok=True)

temp_dir = tempfile.TemporaryDirectory(dir=juv_data_dir)
merged_dir = Path(temp_dir.name)

def handle_termination(signum, frame):
    temp_dir.cleanup()
    sys.exit(0)

signal.signal(signal.SIGTERM, handle_termination)
signal.signal(signal.SIGINT, handle_termination)

config_paths = []
root_data_dir = Path(sys.prefix) / "share" / "jupyter"
jupyter_paths = [root_data_dir]
for path in map(Path, sys.path):
    if not path.name == "site-packages":
        continue
    venv_path = path.parent.parent.parent
    config_paths.append(venv_path / "etc" / "jupyter")
    data_dir = venv_path / "share" / "jupyter"
    if not data_dir.exists() or str(data_dir) == str(root_data_dir):
        continue

    jupyter_paths.append(data_dir)


for path in reversed(jupyter_paths):
    for item in path.rglob('*'):
        if item.is_file():
            dest = merged_dir / item.relative_to(path)
            dest.parent.mkdir(parents=True, exist_ok=True)
            try:
                os.link(item, dest)
            except FileExistsError:
                pass

os.environ["JUPYTER_DATA_DIR"] = str(merged_dir)
os.environ["JUPYTER_CONFIG_PATH"] = os.pathsep.join(map(str, config_paths))"#;
