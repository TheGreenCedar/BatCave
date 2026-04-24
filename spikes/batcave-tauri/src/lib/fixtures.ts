import type { ProcessSample, RuntimeSnapshot } from "./types";

const names = [
  "Code.exe",
  "msedge.exe",
  "BatCave.App.exe",
  "WindowsTerminal.exe",
  "MsMpEng.exe",
  "explorer.exe",
  "SearchHost.exe",
  "dwm.exe",
  "powershell.exe",
  "OneDrive.exe",
  "Rider64.exe",
  "Teams.exe",
];

const baseTs = Date.now();

export function makeFixtureSnapshot(tick: number): RuntimeSnapshot {
  const cpu = wave(tick, 31, 14, 0.34);
  const memoryTotal = 32 * 1024 * 1024 * 1024;
  const memoryRatio = 0.55 + Math.sin(tick / 14) * 0.06;
  const logicalCpu = Array.from({ length: 12 }, (_, index) =>
    clamp(wave(tick + index * 2, 24 + index * 1.5, 20, 0.22 + index * 0.017), 1, 96),
  );
  const processes = names
    .map((name, index) => makeProcess(name, index, tick))
    .sort((left, right) => right.cpu_percent - left.cpu_percent);

  return {
    event_kind: "runtime_snapshot",
    seq: tick,
    ts_ms: baseTs + tick * 1000,
    source: "fixture",
    health: {
      tick_count: tick,
      snapshot_latency_ms: 3 + Math.round(Math.abs(Math.sin(tick / 3)) * 9),
      degraded: false,
      collector_warnings: 0,
    },
    system: {
      cpu_percent: clamp(cpu, 0, 100),
      kernel_cpu_percent: clamp(cpu * 0.18, 0, 100),
      logical_cpu_percent: logicalCpu,
      memory_used_bytes: Math.round(memoryTotal * memoryRatio),
      memory_total_bytes: memoryTotal,
      swap_used_bytes: Math.round(1.8 * 1024 * 1024 * 1024 + Math.sin(tick / 7) * 220_000_000),
      swap_total_bytes: 8 * 1024 * 1024 * 1024,
      process_count: 284 + Math.round(Math.sin(tick / 8) * 8),
      disk_read_total_bytes:
        62_000_000_000 + tick * 11_700_000 + Math.round(Math.sin(tick / 2) * 3_000_000),
      disk_write_total_bytes:
        39_000_000_000 + tick * 7_900_000 + Math.round(Math.cos(tick / 4) * 1_700_000),
      network_received_total_bytes:
        512_000_000_000 + tick * 2_900_000 + Math.round(Math.sin(tick / 3) * 900_000),
      network_transmitted_total_bytes:
        188_000_000_000 + tick * 1_400_000 + Math.round(Math.cos(tick / 5) * 500_000),
    },
    processes,
    warnings: [],
  };
}

export function nextFixtureSnapshot(previousTick: number): RuntimeSnapshot {
  return makeFixtureSnapshot(previousTick);
}

function makeProcess(name: string, index: number, tick: number): ProcessSample {
  const pid = 2100 + index * 137;
  const baseCpu = index === 2 ? 18 : 2 + index * 1.7;
  const cpu = clamp(wave(tick + index, baseCpu, 9, 0.42 + index * 0.03), 0.1, 72);
  const memory = (180 + index * 74 + Math.sin(tick / 5 + index) * 42) * 1024 * 1024;

  return {
    pid: `${pid}`,
    parent_pid: index < 2 ? null : `${1800 + index * 19}`,
    name,
    exe: `C:\\Program Files\\${name.replace(".exe", "")}\\${name}`,
    status: index % 5 === 0 ? "Run" : "Sleep",
    cpu_percent: round1(cpu),
    memory_bytes: Math.max(18 * 1024 * 1024, Math.round(memory)),
    virtual_memory_bytes: Math.round(memory * 1.7),
    disk_read_total_bytes: 12_000_000_000 + index * 500_000_000 + tick * (70_000 + index * 9_000),
    disk_write_total_bytes: 8_000_000_000 + index * 290_000_000 + tick * (45_000 + index * 7_000),
  };
}

function wave(tick: number, base: number, amplitude: number, speed: number): number {
  return (
    base + Math.sin(tick * speed) * amplitude + Math.cos(tick * speed * 0.43) * amplitude * 0.35
  );
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

function round1(value: number): number {
  return Math.round(value * 10) / 10;
}
