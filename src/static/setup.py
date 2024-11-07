####################################################################################################

# This script is embedded into the dynamically generated main script in src/script.rs.
# It is used to setup the Jupyter environment for the main script (mergeing jupyter-dirs
# of uv's from multiple virtual environments).


def setup_merged_jupyter_environment():
    """Setup Jupyter data directories and config paths from multiple virtual environments."""
    import os
    import signal
    import sys
    import tempfile
    from pathlib import Path

    # jupyterlab, notebook, and nbclassic have this as a dependency
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
        for item in path.rglob("*"):
            if item.is_file():
                dest = merged_dir / item.relative_to(path)
                dest.parent.mkdir(parents=True, exist_ok=True)
                try:
                    os.link(item, dest)
                except FileExistsError:
                    pass

    os.environ["JUPYTER_DATA_DIR"] = str(merged_dir)
    os.environ["JUPYTER_CONFIG_PATH"] = os.pathsep.join(map(str, config_paths))


def setup():
    """Setup the Jupyter environment. Called from the main script."""

    setup_merged_jupyter_environment()


####################################################################################################
