import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";

const tauriRoot = new URL("../src-tauri/", import.meta.url);

async function text(path) {
  return readFile(new URL(path, tauriRoot), "utf8");
}

test("Windows bundle packages the collector service beside the asInvoker desktop", async () => {
  const config = JSON.parse(await text("tauri.windows.conf.json"));
  assert.equal(config.build, undefined);
  assert.equal(config.bundle.resources, undefined);
  assert.equal(config.bundle.windows.nsis.installMode, "perMachine");
  assert.equal(config.bundle.windows.nsis.installerHooks, "windows/nsis-hooks.nsh");
  assert.match(await text("src/bin/batcave-collector-service.rs"), /run_collector_service/u);

  const manifest = await text("release.manifest.xml");
  assert.match(manifest, /requestedExecutionLevel level="asInvoker"/u);
  assert.doesNotMatch(manifest, /requireAdministrator|highestAvailable/u);
});

test("NSIS hooks own only the fixed LocalSystem collector service", async () => {
  const hooks = await text("windows/nsis-hooks.nsh");
  for (const hook of ["NSIS_HOOK_PREINSTALL", "NSIS_HOOK_POSTINSTALL", "NSIS_HOOK_PREUNINSTALL"]) {
    assert.match(hooks, new RegExp(`!macro ${hook}\\b`, "u"));
  }

  assert.match(hooks, /BatCaveInstallerOwner/u);
  assert.match(hooks, /dev\.batcave\.monitor\/service-v1/u);
  assert.match(hooks, /RegOpenKeyExW/u);
  assert.match(hooks, /malformed \$\{BATCAVE_SERVICE_NAME\} service registration/u);
  assert.match(hooks, /ImagePath/u);
  assert.match(hooks, /ObjectName/u);
  assert.match(hooks, /unexpected service type/u);
  assert.match(hooks, /\$PROGRAMFILES64\\BatCave Monitor/u);
  assert.match(hooks, /--provision prepare-upgrade/u);
  assert.match(hooks, /--provision install/u);
  assert.match(hooks, /--provision uninstall/u);
  assert.equal(hooks.match(/!insertmacro CheckIfAppIsRunning/gmu)?.length, 2);
  assert.doesNotMatch(hooks, /ExecShell|batcave-monitor\.exe.*start/u);
});

test("NSIS hooks delegate all privileged mutation to fixed native verbs", async () => {
  const hooks = await text("windows/nsis-hooks.nsh");
  assert.match(
    hooks,
    /nsExec::ExecToStack `"\$INSTDIR\\\$\{BATCAVE_SERVICE_BINARY\}" \$\{VERB\}`/u,
  );
  assert.doesNotMatch(
    hooks,
    /\b(?:sc\.exe|icacls\.exe|powershell\.exe|cmd\.exe|CreateDirectory|RMDir|DeleteReg|WriteReg)\b/iu,
  );
  assert.doesNotMatch(hooks, /\$COMMONAPPDATA|ProgramData/u);
  assert.doesNotMatch(hooks, /NSIS_HOOK_POSTUNINSTALL/u);

  const preinstall = hooks.slice(
    hooks.indexOf("!macro NSIS_HOOK_PREINSTALL"),
    hooks.indexOf("!macro NSIS_HOOK_POSTINSTALL"),
  );
  assert.ok(
    preinstall.indexOf("CheckIfAppIsRunning") < preinstall.indexOf("--provision prepare-upgrade"),
  );
  const preuninstall = hooks.slice(hooks.indexOf("!macro NSIS_HOOK_PREUNINSTALL"));
  assert.ok(
    preuninstall.indexOf("CheckIfAppIsRunning") < preuninstall.indexOf("--provision uninstall"),
  );
});
