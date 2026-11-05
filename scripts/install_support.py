"""Platform policy for installation; no installation side effects on import."""
import os
from pathlib import Path
import time


def install_paths(home, environment, windows=None):
    windows = os.name == "nt" if windows is None else windows
    configured = environment.get("HERDR_APP_HOME")
    if windows and not configured:
        raise ValueError("Set HERDR_APP_HOME to your native Herdr app home; its Windows default has not been verified")
    app_home = Path(configured or home / ".config/herdr-app").expanduser().resolve()
    binary = home / ".local/bin" / ("herdr-annotator.exe" if windows else "herdr-annotator")
    return app_home, binary, home / ".local/share/herdr-annotator/install.json"


def replace_helper(source, binary, stamp, windows=None):
    """Bound sharing-violation retries; failure leaves the old helper untouched."""
    import shutil
    windows = os.name == "nt" if windows is None else windows
    temp = binary.with_name(f".{binary.stem}-{stamp}{binary.suffix}")
    try:
        shutil.copy2(source, temp)
        for attempt in range(21):
            try:
                os.replace(temp, binary)
                return
            except PermissionError as error:
                if not windows or attempt == 20:
                    raise PermissionError(f"Cannot replace {binary}; close running helper processes and retry installation") from error
                time.sleep(0.1)
    finally:
        temp.unlink(missing_ok=True)
