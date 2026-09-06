export type DetailMode = "cpu" | "memory" | "disk" | "network";

export interface ResourceSummaryOption {
  mode: DetailMode;
  ariaLabel: string;
  label: string;
  value: string;
  supportingMetrics: { label: string; value: string }[];
  statusLabel: string;
  shortStatusLabel: string;
  values: number[];
  max: number;
  stroke: string;
  fill: string;
}
