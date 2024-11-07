use std::{borrow::Cow, path::Path, str::FromStr};

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
    /// Provides the executable name for the runtime
    fn exacutable(&self) -> &'static str {
        match self.kind {
            RuntimeKind::Notebook => "jupyter-notebook",
            RuntimeKind::Lab => "jupyter-lab",
            RuntimeKind::Nbclassic => "jupyter-nbclassic",
        }
    }

    /// Provides the module specifer to import the main function for the runtime
    fn main_import(&self) -> &'static str {
        if self.kind == RuntimeKind::Notebook && self.version.as_deref() == Some("6") {
            return "notebook.notebookapp";
        };
        match self.kind {
            RuntimeKind::Notebook => "notebook.app",
            RuntimeKind::Lab => "jupyterlab.labapp",
            RuntimeKind::Nbclassic => "nbclassic.notebookapp",
        }
    }

    /// Provides the package name for the runtime
    fn package_name(&self) -> &'static str {
        match self.kind {
            RuntimeKind::Notebook => "notebook",
            RuntimeKind::Lab => "jupyterlab",
            RuntimeKind::Nbclassic => "nbclassic",
        }
    }

    /// Provides the with args for the Runtime for uv --with=...
    pub fn with_args(&self) -> Cow<'static, str> {
        let specifier = if let Some(version) = &self.version {
            Cow::Owned(format!("{}=={}", self.package_name(), version))
        } else {
            Cow::Borrowed(self.package_name())
        };
        if self.kind == RuntimeKind::Notebook && self.version.as_deref() == Some("6") {
            // notebook v6 requires setuptools
            format!("{},setuptools", specifier).into()
        } else {
            specifier
        }
    }

    /// Dynamically generates a script for uv to run the notebook/lab/nbclassic in an isolated environment
    pub fn prepare_run_script(
        &self,
        path: &Path,
        meta: Option<&str>,
        is_managed: bool,
        jupyter_args: &[String],
    ) -> String {
        let notebook = path.to_string_lossy();
        let mut args: Vec<&str> = vec![self.exacutable(), notebook.as_ref()];
        args.extend(jupyter_args.iter().map(String::as_str));

        let print_version: Cow<'static, str> = if is_managed {
            format!(
                r#"import importlib.metadata;print("JUV_MANGED=" + "{name}" + "," + importlib.metadata.version("{name}"), file=sys.stderr)"#,
                name = self.package_name()
            )
            .into()
        } else {
            // only print version if we are in the managed mode
            "".into()
        };

        format!(
            r#"{meta}

{setup_script}

def run():
    import sys
    from {main_import} import main

    setup()
    {print_version}
    sys.argv = {sys_argv}
    main()

if __name__ == "__main__":
    run()"#,
            meta = meta.unwrap_or(""),
            setup_script = include_str!("static/setup.py"),
            main_import = self.main_import(),
            print_version = print_version,
            sys_argv = format!("{:?}", args)
        )
    }
}
