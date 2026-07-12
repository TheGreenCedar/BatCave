export type DetailMode = "cpu" | "memory" | "disk" | "network";

export interface ResourceSummaryOption {
  mode: DetailMode;
  ariaLabel: string;
  label: string;
  value: string;
  supportingLabel: string;
  supportingValue: string;
  statusLabel: string;
  shortStatusLabel: string;
  values: number[];
  max: number;
  stroke: string;
  fill: string;
}
