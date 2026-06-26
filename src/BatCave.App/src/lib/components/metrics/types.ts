export type DetailMode = "cpu" | "memory" | "disk" | "network";

export interface MetricCardOption {
  mode: DetailMode;
  ariaLabel: string;
  label: string;
  value: string;
  sublabel: string;
  values: number[];
  max: number;
  stroke: string;
  fill: string;
  contrastValue: number;
}
