import assert from "node:assert/strict";
import { createHash } from "node:crypto";
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

function between(source, start, end) {
  const startIndex = source.indexOf(start);
  const endIndex = source.indexOf(end, startIndex + start.length);
  assert.notEqual(startIndex, -1, `missing ${start}`);
  assert.notEqual(endIndex, -1, `missing ${end}`);
  return source.slice(startIndex, endIndex);
}

function assertOrdered(source, ...needles) {
  let previous = -1;
  for (const needle of needles) {
    const index = source.indexOf(needle);
    assert.notEqual(index, -1, `missing ordered token: ${needle}`);
    assert.ok(index > previous, `out-of-order token: ${needle}`);
    previous = index;
  }
}

function assertExactLine(source, line, count = 1) {
  assert.equal(
    source.split(/\r?\n/u).filter((candidate) => candidate.trim() === line).length,
    count,
    `expected ${count} exact line(s): ${line}`,
  );
}

test("Windows bundle packages the collector service beside the asInvoker desktop", async () => {
  const config = JSON.parse(await text("tauri.windows.conf.json"));
  assert.equal(config.build, undefined);
  assert.equal(config.bundle.resources, undefined);
  assert.equal(config.bundle.windows.nsis.installMode, "perMachine");
  assert.equal(config.bundle.windows.nsis.installerHooks, "windows/nsis-hooks.nsh");
  assert.equal(config.bundle.windows.nsis.template, "windows/installer-template.nsi");
  assert.match(await text("src/bin/batcave-collector-service.rs"), /run_collector_service/u);

  const manifest = await text("release.manifest.xml");
  assert.match(manifest, /requestedExecutionLevel level="asInvoker"/u);
  assert.doesNotMatch(manifest, /requireAdministrator|highestAvailable/u);
});

test("NSIS hooks own only fixed native service and shortcut retirement verbs", async () => {
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
  assert.match(hooks, /--provision retire-installer-shortcuts/u);
  assert.match(hooks, /--provision uninstall/u);
  assert.match(hooks, /--provision uninstall-staged/u);
  assert.doesNotMatch(
    hooks,
    /\b(?:CreateShortcut|SetShortcutTarget|IsShortcutTarget|UnpinShortcut|SetLnkAppUserModelId|MUI_STARTMENU_GETFOLDER)\b/u,
    "hooks must not inspect or mutate a shared shortcut directly",
  );
  assert.doesNotMatch(
    hooks,
    /\$(?:DESKTOP|SMPROGRAMS)\b/u,
    "hooks must not address a shared shortcut path directly",
  );
  assert.equal(hooks.match(/!insertmacro CheckIfAppIsRunning/gmu)?.length, 2);
  assert.doesNotMatch(hooks, /ExecShell|batcave-monitor\.exe.*start/u);
});

test("native provisioner exclusively owns the fixed shared App Paths lifecycle", async () => {
  const hooks = await text("windows/nsis-hooks.nsh");
  const provisioner = await text("src/collector_service/windows_provisioner.rs");
  const exactPath = String.raw`Software\Microsoft\Windows\CurrentVersion\App Paths\batcave-monitor.exe`;

  assert.doesNotMatch(hooks, /App Paths|RegCreateKeyExW|RegSetValueExW|NtDeleteKey/u);
  assert.match(provisioner, new RegExp(exactPath.replaceAll("\\", "\\\\"), "u"));
  for (const token of [
    "RegCreateKeyExW",
    "NtDeleteKey",
    "KEY_WOW64_64KEY",
    "APP_PATH_SECURITY_SDDL",
    "preflight_app_path_registration",
    "ensure_app_path_registration",
    "remove_app_path_registration",
  ]) {
    assert.match(provisioner, new RegExp(token, "u"));
  }

  const install = between(provisioner, "pub(super) fn install()", "fn prepare_upgrade_transaction");
  assertOrdered(
    install,
    "verify_monitor_image(&image)",
    "preflight_app_path_registration(&monitor)",
    "open_manager",
    "ensure_app_path_registration(&monitor)",
    "rollback_new_install",
  );
  const uninstall = between(
    provisioner,
    "fn uninstall_with_controller(",
    "fn finish_uninstall_after_service_absent(",
  );
  assertOrdered(
    uninstall,
    "verify_monitor_image(controller)",
    "preflight_app_path_registration(&monitor)",
    "open_manager",
    "wait_service_deleted(&manager)",
    "remove_app_path_registration(app_path, monitor.path())",
  );
});

test("NSIS disables and retires both shared Tauri shortcut surfaces", async () => {
  const hooks = await text("windows/nsis-hooks.nsh");
  const installerTemplate = await text("windows/installer-template.nsi");
  const packageJson = JSON.parse(await repoText("src/BatCave.App/package.json"));
  const provisioner = await text("src/collector_service/windows_provisioner.rs");
  const retirement = await text("src/collector_service/windows_shortcut_retirement.rs");

  assert.equal(packageJson.devDependencies["@tauri-apps/cli"], "2.11.4");
  for (const name of ["CreateOrUpdateStartMenuShortcut", "CreateOrUpdateDesktopShortcut"]) {
    const body = between(installerTemplate, `Function ${name}`, "FunctionEnd");
    assertOrdered(body, "${If} $NoShortcutMode = 1", "Return", "${EndIf}", "IsShortcutTarget");
    assert.doesNotMatch(
      body.slice(0, body.indexOf("${If} $NoShortcutMode = 1")),
      /IsShortcutTarget|SetShortcutTarget|CreateDirectory|CreateShortcut|SetLnkAppUserModelId|\bReturn\b/u,
    );
  }
  assert.doesNotMatch(installerTemplate, /^!define MUI_FINISHPAGE_SHOWREADME(?:_|$)/gmu);
  assert.doesNotMatch(
    installerTemplate,
    /MUI_FINISHPAGE_SHOWREADME_FUNCTION CreateOrUpdateDesktopShortcut/u,
  );
  const uninstall = between(installerTemplate, "Section Uninstall", "SectionEnd");
  const shortcutRemoval = between(
    uninstall,
    "; Remove shortcuts if not updating",
    "; Remove registry information for add/remove programs",
  );
  assertOrdered(
    shortcutRemoval,
    "${If} $UpdateMode <> 1",
    "DeleteAppUserModelId",
    "${If} $NoShortcutMode <> 1",
    "IsShortcutTarget",
  );
  assert.equal(shortcutRemoval.match(/DeleteAppUserModelId/gmu)?.length, 1);
  const recoveryRetirement =
    '!insertmacro BATCAVE_RETIRE_SHARED_SHORTCUTS "$INSTDIR\\batcave-collector-service.recovery.exe"';
  const stableRetirement =
    '!insertmacro BATCAVE_RETIRE_SHARED_SHORTCUTS "$INSTDIR\\${BATCAVE_SERVICE_BINARY}"';
  const compatibilityRetirement =
    '!insertmacro BATCAVE_RETIRE_SHARED_SHORTCUTS "$INSTDIR\\batcave-collector-service.${VERSION}.staged.exe"';

  const preinstall = between(hooks, "!macro NSIS_HOOK_PREINSTALL", "!macro NSIS_HOOK_POSTINSTALL");
  assertExactLine(preinstall, "StrCpy $NoShortcutMode 1");
  assertExactLine(preinstall, "StrCpy $WixMode 0");
  assertExactLine(preinstall, recoveryRetirement);
  assertOrdered(
    preinstall,
    "StrCpy $NoShortcutMode 1",
    "StrCpy $WixMode 0",
    recoveryRetirement,
    "--provision prepare-upgrade-staged",
  );

  const postinstall = between(
    hooks,
    "!macro NSIS_HOOK_POSTINSTALL",
    "!macro NSIS_HOOK_PREUNINSTALL",
  );
  const upgradeBranch = between(postinstall, '${If} $BatCaveServiceUpgrade == "1"', "${Else}");
  const freshBranch = between(postinstall, "${Else}", "${EndIf}");
  assertExactLine(upgradeBranch, recoveryRetirement);
  assertExactLine(freshBranch, stableRetirement);
  assertOrdered(upgradeBranch, "--provision commit-upgrade-staged", recoveryRetirement);
  assertOrdered(postinstall, "${EndIf}", "--provision install");

  const preuninstall = hooks.slice(hooks.indexOf("!macro NSIS_HOOK_PREUNINSTALL"));
  assertExactLine(preuninstall, "StrCpy $NoShortcutMode 1");
  const recoveryUninstall = between(
    preuninstall,
    "use_recovery_uninstall_service_binary:",
    "use_legacy_staged_uninstall_service_binary:",
  );
  const compatibilityUninstall = between(
    preuninstall,
    "use_legacy_staged_uninstall_service_binary:",
    "use_stable_uninstall_service_binary:",
  );
  const stableUninstall = between(
    preuninstall,
    "use_stable_uninstall_service_binary:",
    "staged_uninstall_service_binary_present:",
  );
  assertExactLine(recoveryUninstall, recoveryRetirement);
  assertExactLine(compatibilityUninstall, compatibilityRetirement);
  assertExactLine(stableUninstall, stableRetirement);
  assertOrdered(preuninstall, "StrCpy $NoShortcutMode 1", recoveryRetirement);
  assertOrdered(recoveryUninstall, recoveryRetirement, "--provision uninstall-staged");
  assertOrdered(compatibilityUninstall, compatibilityRetirement, "--provision uninstall-staged");
  assertOrdered(stableUninstall, stableRetirement, "--provision uninstall");

  assert.match(provisioner, /"retire-installer-shortcuts"/u);
  assert.match(
    provisioner,
    /ProvisionVerb::RetireInstallerShortcuts => native::retire_installer_shortcuts\(\)/u,
  );
  assert.match(provisioner, /retire_shared_legacy_shortcuts/u);
  assert.match(provisioner, /InstallerControllerKind::Stable => verify_current_binary_path\(\)/u);
  assert.match(
    provisioner,
    /InstallerControllerKind::Staged => verify_current_staged_binary_path\(\)/u,
  );
  assert.match(provisioner, /install_directory\(\)\?[\s\S]*?\.join\(MONITOR_EXECUTABLE_NAME\)/u);
  assert.match(retirement, /FOLDERID_PublicDesktop/u);
  assert.match(retirement, /FOLDERID_CommonPrograms/u);
  assert.doesNotMatch(retirement, /C:\\Users\\Public|C:\\ProgramData|C:\\Program Files/iu);
  assert.match(retirement, /const SHORTCUT_NAME: &str = "BatCave Monitor\.lnk"/u);
  assert.match(retirement, /FILE_FLAG_OPEN_REPARSE_POINT/u);
  const pinnedShortcutOpen = between(
    retirement,
    "impl PinnedShortcut",
    "pub(super) fn retire_shared_legacy_shortcuts",
  );
  assert.match(pinnedShortcutOpen, /FILE_SHARE_READ,/u);
  assert.doesNotMatch(pinnedShortcutOpen, /FILE_SHARE_WRITE|FILE_SHARE_DELETE/u);
  assert.match(retirement, /hardlink_count != 1/u);
  assert.match(retirement, /ReadFile/u);
  assert.match(retirement, /SHCreateMemStream/u);
  assert.match(retirement, /IID_PERSIST_STREAM/u);
  assert.match(retirement, /SLGP_RAWPATH/u);
  assert.match(retirement, /IID_PROPERTY_STORE/u);
  assert.match(retirement, /SetFileInformationByHandle/u);
  assert.match(retirement, /FileDispositionInfo/u);
  assert.doesNotMatch(
    retirement,
    /DeleteFileW|RemoveDirectoryW|SetNamedSecurityInfo|SetSecurityInfo/u,
  );
  assert.doesNotMatch(retirement, /ProfileList|HKEY_USERS|FOLDERID_Programs|LocalAppData/u);
});

test("pinned NSIS template differs from Tauri 2.11.4 only by audited BatCave deltas", async () => {
  const installerTemplate = await text("windows/installer-template.nsi");
  const header = [
    "; Pinned from Tauri CLI 2.11.4 (tauri-bundler 2.9.4, commit 8909f221d1515955fc843808032bdc5d62209c96).",
    "; BatCave deltas are limited to shortcut suppression in install, finish, and uninstall paths.",
    "",
  ].join("\n");
  const uninstallerExport = [
    "; Export exact post-sign uninstaller bytes only for lifecycle-proof artifact builds.",
    '!if "$%BATCAVE_UNINSTALLER_EXPORT_PATH%" != ""',
    '  !uninstfinalize \'"$%ComSpec%" /D /C copy /Y "%1" "$%BATCAVE_UNINSTALLER_EXPORT_PATH%"\' = 0',
    "!endif",
    "",
    "",
  ].join("\n");
  const guard = [
    "  ; BatCave owns no shared shortcuts. Guard before Tauri's legacy migration paths.",
    "  ${If} $NoShortcutMode = 1",
    "    Return",
    "  ${EndIf}",
    "",
    "",
  ].join("\n");
  const finishControlOmission =
    "; BatCave owns no installer-created shortcut, so the stock desktop-shortcut control is omitted.\n";
  const upstreamFinishControl = [
    "; Use show readme button in the finish page as a button create a desktop shortcut",
    "!define MUI_FINISHPAGE_SHOWREADME",
    '!define MUI_FINISHPAGE_SHOWREADME_TEXT "$(createDesktop)"',
    "!define MUI_FINISHPAGE_SHOWREADME_FUNCTION CreateOrUpdateDesktopShortcut",
    "",
  ].join("\n");
  const uninstallGuardComment =
    "    ; BatCave's native PREUNINSTALL authority already retired both exact shared links.";
  const uninstallGuardStart = "    ${If} $NoShortcutMode <> 1";

  assert.ok(installerTemplate.startsWith(header));
  assert.equal(installerTemplate.split(guard).length - 1, 2);
  assert.equal(installerTemplate.split(finishControlOmission).length - 1, 1);
  assert.equal(installerTemplate.split(uninstallGuardComment).length - 1, 1);
  assert.equal(installerTemplate.split(uninstallGuardStart).length - 1, 1);
  assert.equal(installerTemplate.split(uninstallerExport).length - 1, 1);
  assert.ok(
    installerTemplate.indexOf(uninstallerExport) >
      installerTemplate.indexOf("!uninstfinalize '${UNINSTALLERSIGNCOMMAND}'"),
    "lifecycle-proof export must capture the post-sign bytes embedded by WriteUninstaller",
  );
  const shortcutRemoval = between(
    installerTemplate,
    "  ; Remove shortcuts if not updating",
    "  ; Remove registry information for add/remove programs",
  );
  const shortcutLines = shortcutRemoval.split("\n");
  const guardCommentIndex = shortcutLines.indexOf(uninstallGuardComment);
  const guardStartIndex = shortcutLines.indexOf(uninstallGuardStart);
  const outerGuardEndIndex = shortcutLines.lastIndexOf("  ${EndIf}");
  const pathGuardEndIndex = shortcutLines.lastIndexOf("    ${EndIf}", outerGuardEndIndex - 1);
  assert.equal(guardStartIndex, guardCommentIndex + 1);
  assert.ok(pathGuardEndIndex > guardStartIndex);
  assert.ok(outerGuardEndIndex > pathGuardEndIndex);
  const upstreamShortcutRemoval = [
    ...shortcutLines.slice(0, guardCommentIndex),
    ...shortcutLines
      .slice(guardStartIndex + 1, pathGuardEndIndex)
      .map((line) => (line.startsWith("  ") ? line.slice(2) : line)),
    ...shortcutLines.slice(pathGuardEndIndex + 1),
  ].join("\n");
  const upstream = installerTemplate
    .slice(header.length)
    .replace(uninstallerExport, "")
    .replaceAll(guard, "")
    .replace(finishControlOmission, upstreamFinishControl)
    .replace(shortcutRemoval, upstreamShortcutRemoval);
  assert.equal(
    createHash("sha256").update(upstream).digest("hex"),
    "20f4ecc730defb71f1342eaeaec4021df13be3d843abba0effe88ea5835fa079",
    "update from the pinned Tauri source and re-audit every shortcut delta before changing this digest",
  );
});

test("Windows validation and release verify the generated NSIS contract", async () => {
  const verifier = await repoText(
    "src/BatCave.App/scripts/windows-installer-generated-contract.mjs",
  );
  const validation = await repoText("scripts/validate-tauri.ps1");
  const release = (await repoText(".github/workflows/release.yml")).replaceAll("\r\n", "\n");
  const releaseInstaller = "src-tauri/target/release/nsis/x64/installer.nsi";
  const releaseBuild = "npm run tauri -- build --config src-tauri/tauri.updater.conf.json";
  const releaseVerification =
    'npm run verify:windows-installer-generated -- "src-tauri/target/release/nsis/x64/installer.nsi"';

  assert.match(verifier, new RegExp(releaseInstaller.replaceAll("/", "\\/"), "u"));
  assert.match(verifier, /generated NSIS must delegate App Paths ownership/u);
  assertOrdered(
    validation,
    "npm run tauri -- build",
    "npm run verify:windows-installer-generated",
    releaseInstaller,
  );
  const windowsReleaseJob = between(release, "\n  windows:\n", "\n  linux:\n");
  assert.equal(
    windowsReleaseJob.match(
      /^\s*(?:run:\s*)?(?:npm run tauri --|npx tauri|cargo tauri|tauri)\s+build\b/gmu,
    )?.length,
    1,
    "the Windows release job must contain exactly one Tauri bundle build",
  );
  assert.equal(
    windowsReleaseJob.split(releaseVerification).length - 1,
    1,
    "the Windows release job must verify the generated installer exactly once",
  );
  assertOrdered(
    windowsReleaseJob,
    "name: Bundle signed Windows updater",
    releaseBuild,
    "name: Verify generated release NSIS shortcut contract",
    releaseVerification,
    "name: Collect Windows distributables",
  );
  const verificationEnd =
    windowsReleaseJob.indexOf(releaseVerification) + releaseVerification.length;
  const collectionStart = windowsReleaseJob.indexOf("      - name: Collect Windows distributables");
  assert.match(
    windowsReleaseJob.slice(verificationEnd, collectionStart),
    /^\s*$/u,
    "collection must immediately follow verification so no later build or installer mutation can overwrite the verified bytes",
  );
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

test("shortcut authority and lifecycle recovery gates are transactionally ordered", async () => {
  const provisioner = await text("src/collector_service/windows_provisioner.rs");

  const wrapper = between(
    provisioner,
    "pub(super) fn retire_installer_shortcuts",
    "pub(super) fn prepare_upgrade",
  );
  assertOrdered(
    wrapper,
    "require_elevated()?",
    "verify_current_installer_controller()?",
    "retire_shortcuts_with_controller(&controller)",
  );
  assertOrdered(wrapper, "drop(controller)", "fail_shortcut_retirement_with_upgrade_recovery");

  const prepare = between(
    provisioner,
    "fn prepare_upgrade_staged_image",
    "pub(super) fn commit_upgrade_staged",
  );
  assertOrdered(
    prepare,
    "verify_monitor_image(staged)?",
    "preflight_app_path_registration(&monitor)?",
    "resume_upgrade_transaction(staged, &monitor, stable, service)?",
    'trusted_file_digest(stable, "collector_service_stable_image")?',
    "ensure_service_generation_ready(service, stable, prior_digest)?",
    "retire_shortcuts_with_controller(staged)?",
    "ensure_uninstaller_compatibility_alias(staged)?",
    "settle_service_for_replacement(service)?",
  );
  assert.equal(prepare.match(/preflight_app_path_registration\(&monitor\)/gmu)?.length, 2);
  assert.equal(
    prepare.match(/ensure_service_generation_ready\(service, stable, prior_digest\)/gmu)?.length,
    2,
    "prepare failure must restart and revalidate the prior generation",
  );

  const resume = between(provisioner, "fn resume_upgrade_transaction", "fn upgrade_resume_action");
  assert.match(
    resume,
    /UpgradeResumeAction::ReusePrepared[\s\S]*ensure_service_generation_ready\(service, stable, journal\.old_digest\)/u,
  );
  assert.match(
    resume,
    /UpgradeResumeAction::CommitCandidate[\s\S]*commit_upgrade_candidate[\s\S]*app_path_upgrade_gate\(staged, monitor\)/u,
  );
  assert.match(
    resume,
    /UpgradeResumeAction::FinalizeVerified[\s\S]*app_path_upgrade_gate\(staged, monitor\)[\s\S]*rollback_upgrade[\s\S]*finalize_verified_upgrade/u,
  );

  const commit = between(provisioner, "fn commit_upgrade_candidate", "fn rollback_upgrade");
  assertOrdered(
    commit,
    "settle_service_for_replacement(service)?",
    "journal.phase = UpgradePhase::CandidateInstalled",
    "write_upgrade_journal(journal)?",
    "start_upgrade_service_generation(service, false)?",
    "journal.phase = UpgradePhase::Verified",
  );
  const verifiedCommit = commit.slice(commit.indexOf("journal.phase = UpgradePhase::Verified"));
  assertOrdered(
    verifiedCommit,
    "write_upgrade_journal(journal)?",
    "final_gate()",
    "rollback_upgrade(journal, service, stable)",
  );

  const install = between(provisioner, "pub(super) fn install", "fn prepare_upgrade_transaction");
  assertOrdered(
    install,
    "start_service_and_wait(&service)?",
    "app_path_upgrade_gate(&image, &monitor)",
    "fail_shortcut_retirement_with_upgrade_recovery",
    "finalize_upgrade_transaction(&image)?",
  );
  assert.equal(install.match(/app_path_upgrade_gate\(&image, &monitor\)/gmu)?.length, 1);
  assert.equal(install.match(/retire_shortcuts_with_controller\(&image\)/gmu)?.length, 1);
  const freshInstall = install.slice(install.indexOf("create_service(&manager, image.path())?"));
  assertOrdered(
    freshInstall,
    "create_service(&manager, image.path())?",
    "start_service_and_wait(&service)?",
    "retire_shortcuts_with_controller(&image)",
    "rollback_new_install",
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
