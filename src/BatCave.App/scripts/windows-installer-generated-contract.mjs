import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";

const generatedPath = resolve(process.argv[2] ?? "src-tauri/target/release/nsis/x64/installer.nsi");
const generated = await readFile(generatedPath, "utf8");

function section(source, start, end) {
  const startIndex = source.indexOf(start);
  const endIndex = source.indexOf(end, startIndex + start.length);
  assert.notEqual(startIndex, -1, `missing ${start}`);
  assert.notEqual(endIndex, -1, `missing ${end}`);
  return source.slice(startIndex, endIndex);
}

assert.match(generated, /^Var NoShortcutMode$/mu);
assert.match(generated, /!include ".*\\windows\\nsis-hooks\.nsh"/u);
assert.doesNotMatch(generated, /^!define MUI_FINISHPAGE_SHOWREADME(?:_|$)/gmu);
assert.doesNotMatch(generated, /MUI_FINISHPAGE_SHOWREADME_FUNCTION CreateOrUpdateDesktopShortcut/u);

const install = section(generated, "Section Install", "SectionEnd");
const preinstall = install.indexOf("!insertmacro NSIS_HOOK_PREINSTALL");
const startShortcut = install.indexOf("Call CreateOrUpdateStartMenuShortcut");
const desktopShortcut = install.indexOf("Call CreateOrUpdateDesktopShortcut");
const postinstall = install.indexOf("!insertmacro NSIS_HOOK_POSTINSTALL");
assert.ok(preinstall >= 0, "generated installer must insert PREINSTALL");
assert.ok(preinstall < startShortcut, "PREINSTALL must precede Start shortcut handling");
assert.ok(preinstall < desktopShortcut, "PREINSTALL must precede Desktop shortcut handling");
assert.ok(startShortcut < postinstall, "POSTINSTALL must follow Start shortcut handling");
assert.ok(desktopShortcut < postinstall, "POSTINSTALL must follow Desktop shortcut handling");

for (const [name, end] of [
  ["Function CreateOrUpdateStartMenuShortcut", "FunctionEnd"],
  ["Function CreateOrUpdateDesktopShortcut", "FunctionEnd"],
]) {
  const body = section(generated, name, end);
  const noShortcutGate = body.indexOf("${If} $NoShortcutMode = 1");
  const noShortcutGateEnd = body.indexOf("${EndIf}", noShortcutGate);
  const createShortcut = body.indexOf("CreateShortcut");
  assert.ok(noShortcutGate >= 0, `${name} must start with a NoShortcutMode guard`);
  assert.ok(noShortcutGateEnd > noShortcutGate, `${name} must close its NoShortcutMode guard`);
  assert.doesNotMatch(
    body.slice(0, noShortcutGate),
    /IsShortcutTarget|SetShortcutTarget|CreateDirectory|CreateShortcut|SetLnkAppUserModelId|\bReturn\b/u,
    `${name} must not inspect, mutate, or return before the NoShortcutMode guard`,
  );
  assert.match(
    body.slice(noShortcutGate, noShortcutGateEnd),
    /\bReturn\b/u,
    `${name} must return from the NoShortcutMode guard`,
  );
  for (const mutation of [
    "IsShortcutTarget",
    "SetShortcutTarget",
    "CreateDirectory",
    "CreateShortcut",
    "SetLnkAppUserModelId",
  ]) {
    const mutationIndex = body.indexOf(mutation);
    if (mutationIndex >= 0) {
      assert.ok(
        noShortcutGateEnd < mutationIndex,
        `${name} must guard ${mutation} behind NoShortcutMode`,
      );
    }
  }
  assert.ok(noShortcutGate < createShortcut, `${name} must gate shortcut creation`);
}

const preinstallOffset = generated.indexOf("!insertmacro NSIS_HOOK_PREINSTALL");
const postPreinstallWixUses = [...generated.matchAll(/\$WixMode/gmu)]
  .map((match) => match.index)
  .filter((index) => index > preinstallOffset);
assert.equal(
  postPreinstallWixUses.length,
  2,
  "generated post-PREINSTALL WixMode use changed; re-audit the shortcut bypass",
);

const uninstall = section(generated, "Section Uninstall", "SectionEnd");
const preuninstall = uninstall.indexOf("!insertmacro NSIS_HOOK_PREUNINSTALL");
const shortcutRemoval = section(
  uninstall,
  "; Remove shortcuts if not updating",
  "; Remove registry information for add/remove programs",
);
const updateGate = shortcutRemoval.indexOf("${If} $UpdateMode <> 1");
const appUserModelCleanup = shortcutRemoval.indexOf("DeleteAppUserModelId");
const noShortcutGate = shortcutRemoval.indexOf("${If} $NoShortcutMode <> 1");
const outerGateEnd = shortcutRemoval.lastIndexOf("${EndIf}");
const noShortcutGateEnd = shortcutRemoval.lastIndexOf("${EndIf}", outerGateEnd - 1);
const pathOperations = [
  "MUI_STARTMENU_GETFOLDER",
  "IsShortcutTarget",
  "UnpinShortcut",
  'Delete "$SMPROGRAMS',
  'RMDir "$SMPROGRAMS',
  'Delete "$DESKTOP',
];
assert.ok(preuninstall >= 0, "generated uninstaller must insert PREUNINSTALL");
assert.ok(
  preuninstall < uninstall.indexOf("; Remove shortcuts if not updating"),
  "PREUNINSTALL must precede every stock shortcut cleanup path",
);
assert.ok(updateGate >= 0, "generated uninstaller must retain the update cleanup gate");
assert.ok(
  appUserModelCleanup > updateGate && appUserModelCleanup < noShortcutGate,
  "generated uninstaller must preserve AppUserModelId cleanup outside the shared-path guard",
);
assert.equal(
  shortcutRemoval.match(/DeleteAppUserModelId/gmu)?.length,
  1,
  "generated uninstaller must retain exactly one AppUserModelId cleanup",
);
assert.ok(noShortcutGate > updateGate, "generated uninstaller must add the NoShortcutMode gate");
assert.ok(noShortcutGateEnd > noShortcutGate, "generated uninstaller must close its path guard");
assert.ok(outerGateEnd > noShortcutGateEnd, "generated uninstaller must retain its update guard");
for (const operation of pathOperations) {
  const indexes = [
    ...shortcutRemoval.matchAll(new RegExp(operation.replaceAll("$", "\\$&"), "gmu")),
  ].map((match) => match.index);
  assert.ok(indexes.length > 0, `generated uninstaller must retain ${operation}`);
  for (const index of indexes) {
    assert.ok(
      index > noShortcutGate && index < noShortcutGateEnd,
      `generated uninstaller must keep ${operation} inside the shared-path guard`,
    );
  }
}

console.log(`generated Windows installer contract passed: ${generatedPath}`);
