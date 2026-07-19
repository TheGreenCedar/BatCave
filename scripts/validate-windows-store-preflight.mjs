import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

import { parseReleaseTag, verifyWorkspaceReleaseVersion } from "./verify-release-version.mjs";

const HTTPS_URL = /^https:\/\/[^\s]+$/u;
const SHA256 = /^sha256:[0-9a-f]{64}$/u;
const SOURCE_SHA = /^[0-9a-f]{40}$/u;

function fail(message) {
  throw new Error(`Windows Store preflight: ${message}`);
}

function exactKeys(value, expected, owner) {
  if (!value || typeof value !== "object" || Array.isArray(value))
    fail(`${owner} must be an object`);
  const actual = Object.keys(value);
  const missing = expected.find((key) => !actual.includes(key));
  const extra = actual.find((key) => !expected.includes(key));
  if (missing) fail(`${owner}.${missing} is required`);
  if (extra) fail(`${owner}.${extra} is not allowed`);
}

export function validateWindowsStoreListing(listing) {
  exactKeys(
    listing,
    ["schema_version", "product_name", "reserved_name", "publisher", "distribution", "listing"],
    "listing",
  );
  if (listing.schema_version !== 1) fail("schema_version must be 1");
  if (listing.product_name !== "BatCave Monitor" || listing.reserved_name !== "BatCave Monitor") {
    fail("product and reserved names must both be BatCave Monitor");
  }
  if (listing.publisher !== "Albert Najjar") fail("publisher must be Albert Najjar");

  exactKeys(
    listing.distribution,
    [
      "type",
      "architecture",
      "install_scope",
      "offline_installer",
      "silent_parameters",
      "success_exit_codes",
      "update_delivery",
      "versioned_url_template",
    ],
    "listing.distribution",
  );
  const distribution = listing.distribution;
  if (distribution.type !== "exe" || distribution.architecture !== "x64") {
    fail("distribution must be the x64 EXE path");
  }
  if (distribution.install_scope !== "per_machine" || distribution.offline_installer !== true) {
    fail("distribution must retain the per-machine offline installer");
  }
  if (distribution.silent_parameters !== "/S") fail("silent installer parameters must be /S");
  if (JSON.stringify(distribution.success_exit_codes) !== "[0]") {
    fail("success exit codes must contain only 0");
  }
  if (distribution.update_delivery !== "publisher_managed") {
    fail("Store-listed EXE distribution must retain BatCave's manual updater");
  }
  if (
    distribution.versioned_url_template !==
    "https://github.com/TheGreenCedar/BatCave/releases/download/{tag}/BatCave.Monitor_{version}_x64-setup.exe"
  ) {
    fail("versioned URL template must be the exact immutable GitHub Release asset path");
  }

  exactKeys(
    listing.listing,
    [
      "category",
      "short_description",
      "description",
      "features",
      "keywords",
      "support_url",
      "privacy_policy_url",
      "license_url",
    ],
    "listing.listing",
  );
  if (listing.listing.category !== "Utilities & tools") fail("category must be Utilities & tools");
  for (const key of ["short_description", "description"]) {
    if (typeof listing.listing[key] !== "string" || listing.listing[key].trim().length < 20) {
      fail(`${key} must contain prepared Store copy`);
    }
  }
  for (const key of ["features", "keywords"]) {
    if (!Array.isArray(listing.listing[key]) || listing.listing[key].length < 3) {
      fail(`${key} must contain at least three entries`);
    }
  }
  for (const key of ["support_url", "privacy_policy_url", "license_url"]) {
    if (!HTTPS_URL.test(listing.listing[key])) fail(`${key} must be an HTTPS URL`);
  }
  return listing;
}

export function buildWindowsStorePreflightReceipt(listing, tag, installerPath, signingInventory) {
  validateWindowsStoreListing(listing);
  const { version } = parseReleaseTag(tag);
  const expectedName = `BatCave.Monitor_${version}_x64-setup.exe`;
  if (path.basename(installerPath) !== expectedName) {
    fail(`installer must be named ${expectedName}`);
  }
  const bytes = fs.readFileSync(installerPath);
  const sha256 = `sha256:${crypto.createHash("sha256").update(bytes).digest("hex")}`;
  if (!SOURCE_SHA.test(signingInventory?.source_sha ?? "")) {
    fail("signing inventory must contain the exact release source SHA");
  }
  if (signingInventory?.profile !== "production" || signingInventory?.phase !== "final") {
    fail("signing inventory must be the final production profile");
  }
  const record = signingInventory.files?.find(({ name }) => name === expectedName);
  if (!record || record.sha256 !== sha256 || record.disposition !== "generated_signed") {
    fail("installer does not match its final Authenticode inventory record");
  }
  if (!SHA256.test(record.certificate_sha256) || !record.rfc3161_timestamp_utc) {
    fail("installer inventory must retain its leaf certificate and RFC3161 timestamp");
  }
  const url = listing.distribution.versioned_url_template
    .replace("{tag}", encodeURIComponent(tag))
    .replace("{version}", version);
  if (url.includes("latest") || !HTTPS_URL.test(url)) fail("package URL must be immutable HTTPS");

  return {
    schema_version: 1,
    qualification: "source_preflight",
    source_sha: signingInventory.source_sha,
    tag,
    package: {
      name: expectedName,
      sha256,
      url,
      type: "exe",
      architecture: "x64",
      silent_parameters: "/S",
      offline_installer: true,
      authenticode: {
        publisher_subject: record.publisher_subject,
        certificate_sha256: record.certificate_sha256,
        rfc3161_timestamp_utc: record.rfc3161_timestamp_utc,
      },
    },
    limitation: "native_clean_machine_silent_install_pending",
  };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [listingPath, tag, installerPath, signingInventoryPath, outputPath] = process.argv.slice(2);
  if (!listingPath || !tag || !installerPath || !signingInventoryPath || !outputPath) {
    console.error(
      "usage: node scripts/validate-windows-store-preflight.mjs <listing-json> <tag> <installer> <signing-inventory-json> <output-json>",
    );
    process.exit(2);
  }
  try {
    verifyWorkspaceReleaseVersion(tag);
    const receipt = buildWindowsStorePreflightReceipt(
      JSON.parse(fs.readFileSync(listingPath, "utf8")),
      tag,
      installerPath,
      JSON.parse(fs.readFileSync(signingInventoryPath, "utf8")),
    );
    fs.writeFileSync(outputPath, `${JSON.stringify(receipt, null, 2)}\n`, { flag: "wx" });
    console.log(`Windows Store source preflight passed for ${receipt.package.name}`);
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
