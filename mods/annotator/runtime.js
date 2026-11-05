// The installer replaces only this declaration with local installation paths.
export const installation = {};

export function runtimeConfig(home, properties = {}) {
  const configured = { ...installation, ...(properties?.runtime ?? {}) };
  return {
    helper: properties?.helper ?? configured.helper ?? `${home}/.local/bin/herdr-annotator`,
    python: configured.python ?? "python3",
    script: configured.script ?? `${home}/.local/share/herdr-annotator/update.py`,
    updatesEnabled: configured.updatesEnabled !== false,
  };
}
