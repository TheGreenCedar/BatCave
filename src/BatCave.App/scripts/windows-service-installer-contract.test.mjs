import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";

const tauriRoot = new URL("../src-tauri/", import.meta.url);
const repoRoot = new URL("../../../", tauriRoot);

async function text(path) {
  return readFile(new URL(path, tauriRoot), "utf8");
}

async function repoText(path) {
  return readFile(new URL(path, repoRoot), "utf8");
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
  assert.doesNotMatch(hooks, /!macro NSIS_HOOK_POSTUNINSTALL\b/u);

  assert.match(hooks, /BatCaveInstallerOwner/u);
  assert.match(hooks, /dev\.batcave\.monitor\/service-v1/u);
  assert.match(hooks, /RegOpenKeyExW/u);
  assert.match(hooks, /malformed \$\{BATCAVE_SERVICE_NAME\} service registration/u);
  assert.match(hooks, /ImagePath/u);
  assert.match(hooks, /ObjectName/u);
  assert.match(hooks, /unexpected service type/u);
  assert.match(hooks, /\$PROGRAMFILES64\\BatCave Monitor/u);
  assert.match(hooks, /--provision prepare-upgrade-staged/u);
  assert.match(hooks, /--provision commit-upgrade-staged/u);
  assert.match(hooks, /--provision install/u);
  assert.match(hooks, /--provision uninstall/u);
  assert.match(hooks, /--provision uninstall-staged/u);
  assert.equal(hooks.match(/!insertmacro CheckIfAppIsRunning/gmu)?.length, 2);
  assert.doesNotMatch(hooks, /ExecShell|batcave-monitor\.exe.*start/u);
});

test("NSIS hooks delegate all privileged mutation to fixed native verbs", async () => {
  const hooks = await text("windows/nsis-hooks.nsh");
  assert.match(hooks, /nsExec::ExecToStack `"\$\{IMAGE\}" \$\{VERB\}`/u);
  assert.doesNotMatch(
    hooks,
    /\b(?:sc\.exe|icacls\.exe|powershell\.exe|cmd\.exe|CreateDirectory|RMDir|WriteReg)\b/iu,
  );
  assert.doesNotMatch(hooks, /\$COMMONAPPDATA|ProgramData/u);
  const preinstall = hooks.slice(
    hooks.indexOf("!macro NSIS_HOOK_PREINSTALL"),
    hooks.indexOf("!macro NSIS_HOOK_POSTINSTALL"),
  );
  assert.ok(
    preinstall.indexOf("CheckIfAppIsRunning") <
      preinstall.indexOf("--provision prepare-upgrade-staged"),
  );
  assert.ok(
    preinstall.indexOf("/oname=batcave-collector-service.recovery.exe") <
      preinstall.indexOf("--provision prepare-upgrade-staged"),
  );
  const preuninstall = hooks.slice(hooks.indexOf("!macro NSIS_HOOK_PREUNINSTALL"));
  assert.match(preuninstall, /DeleteRegKey HKLM "Software\\batcave\\BatCave Monitor"/u);
  assert.match(preuninstall, /DeleteRegKey \/ifempty HKLM "Software\\batcave"/u);
  assert.equal(preuninstall.match(/\bDeleteRegKey\b/gmu)?.length, 2);
  assert.match(preuninstall, /The BatCave product registry key is still present/u);
  assert.match(preuninstall, /RegOpenKeyExW/u);
  assert.ok(
    preuninstall.indexOf("CheckIfAppIsRunning") < preuninstall.indexOf("--provision uninstall"),
  );
  assert.ok(
    preuninstall.indexOf("--provision uninstall-staged") <
      preuninstall.lastIndexOf("--provision uninstall"),
  );
  assert.ok(
    preuninstall.indexOf("batcave-collector-service.recovery.exe") <
      preuninstall.indexOf("batcave-collector-service.${VERSION}.staged.exe"),
  );
  const recoveryBranch = preuninstall.slice(
    preuninstall.indexOf("use_recovery_uninstall_service_binary:"),
    preuninstall.indexOf("use_legacy_staged_uninstall_service_binary:"),
  );
  assert.ok(
    recoveryBranch.indexOf("--provision uninstall-staged") <
      recoveryBranch.indexOf('Delete "$INSTDIR\\batcave-collector-service.recovery.exe"'),
  );
  assert.ok(
    recoveryBranch.indexOf('Delete "$INSTDIR\\batcave-collector-service.recovery.exe"') <
      recoveryBranch.indexOf('IfFileExists "$INSTDIR\\batcave-collector-service.recovery.exe"'),
  );
  assert.ok(
    recoveryBranch.indexOf('IfFileExists "$INSTDIR\\batcave-collector-service.recovery.exe"') <
      recoveryBranch.indexOf("Goto service_uninstall_complete"),
  );
  const compatibilityBranch = preuninstall.slice(
    preuninstall.indexOf("use_legacy_staged_uninstall_service_binary:"),
    preuninstall.indexOf("use_stable_uninstall_service_binary:"),
  );
  assert.ok(
    compatibilityBranch.indexOf("--provision uninstall-staged") <
      compatibilityBranch.indexOf(
        'Delete "$INSTDIR\\batcave-collector-service.${VERSION}.staged.exe"',
      ),
  );
  assert.ok(
    compatibilityBranch.indexOf(
      'Delete "$INSTDIR\\batcave-collector-service.${VERSION}.staged.exe"',
    ) <
      compatibilityBranch.indexOf(
        'IfFileExists "$INSTDIR\\batcave-collector-service.${VERSION}.staged.exe"',
      ),
  );
  assert.ok(
    compatibilityBranch.indexOf(
      'IfFileExists "$INSTDIR\\batcave-collector-service.${VERSION}.staged.exe"',
    ) < compatibilityBranch.indexOf("Goto service_uninstall_complete"),
  );
  assert.ok(
    preuninstall.indexOf("use_stable_uninstall_service_binary:") >
      preuninstall.indexOf("use_legacy_staged_uninstall_service_binary:"),
  );
  assert.ok(
    preuninstall.indexOf("service_uninstall_complete:") <
      preuninstall.indexOf('DeleteRegKey HKLM "Software\\batcave\\BatCave Monitor"'),
  );
  const postinstall = hooks.slice(
    hooks.indexOf("!macro NSIS_HOOK_POSTINSTALL"),
    hooks.indexOf("!macro NSIS_HOOK_PREUNINSTALL"),
  );
  assert.ok(
    postinstall.indexOf("--provision commit-upgrade-staged") <
      postinstall.indexOf("--provision install"),
  );
});

test("legacy Windows CLI cleanup stays exact and native", async () => {
  const config = JSON.parse(await text("tauri.windows.conf.json"));
  const hooks = await text("windows/nsis-hooks.nsh");
  const provisioner = await text("src/collector_service/windows_provisioner.rs");
  const releaseAssets = await repoText("scripts/release-asset-contract.mjs");

  assert.doesNotMatch(JSON.stringify(config.bundle), /batcave-monitor-cli/iu);
  assert.doesNotMatch(hooks, /batcave-monitor-cli/iu);
  assert.match(releaseAssets, /role: "Windows CLI executable"/u);
  assert.match(releaseAssets, /name: \(\) => "batcave-monitor-cli\.exe"/u);
  assert.match(provisioner, /const LEGACY_WINDOWS_CLI_NAME/u);
  assert.match(provisioner, /const LEGACY_WINDOWS_CLI_IMAGES/u);
  assert.match(provisioner, /SetFileInformationByHandle/u);
  assert.match(provisioner, /FileDispositionInfo/u);
  const prepareUpgrade = provisioner.slice(
    provisioner.indexOf("pub(super) fn prepare_upgrade"),
    provisioner.indexOf("pub(super) fn install"),
  );
  const install = provisioner.slice(
    provisioner.indexOf("pub(super) fn install"),
    provisioner.indexOf("pub(super) fn uninstall"),
  );
  const uninstall = provisioner.slice(
    provisioner.indexOf("pub(super) fn uninstall"),
    provisioner.indexOf("fn open_manager"),
  );
  assert.doesNotMatch(prepareUpgrade, /retire_legacy_cli/u);
  assert.equal(install.match(/retire_legacy_cli\(&image\)/gmu)?.length, 2);
  assert.equal(uninstall.match(/retire_legacy_cli\(controller\)/gmu)?.length, 2);
  assert.equal(install.match(/retire_staged_upgrade_image\(&image\)/gmu)?.length, 2);
  assert.equal(uninstall.match(/retire_staged_upgrade_image\(controller\)/gmu)?.length, 2);
  assert.ok(install.lastIndexOf("retire_legacy_cli") > install.indexOf("start_service_and_wait"));
  assert.ok(
    install.indexOf("retire_upgrade_transaction_for_uninstall(image.path(), None)") <
      install.indexOf("create_service(&manager, image.path())"),
  );
  assert.ok(uninstall.lastIndexOf("retire_legacy_cli") > uninstall.indexOf("wait_service_deleted"));

  const commitCandidate = provisioner.slice(
    provisioner.indexOf("fn commit_upgrade_candidate"),
    provisioner.indexOf("fn rollback_upgrade"),
  );
  assert.ok(
    commitCandidate.indexOf("settle_service_for_replacement(service)") <
      commitCandidate.indexOf("journal.phase = UpgradePhase::CandidateInstalled"),
  );
});
