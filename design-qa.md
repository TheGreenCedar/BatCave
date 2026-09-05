# Design QA

## Source targets

- `artifacts/design/batcave-ui-overhaul/selected-overview-light.png`
- `artifacts/design/batcave-ui-overhaul/selected-explore-light.png`
- `artifacts/design/batcave-ui-overhaul/selected-overview-compact-light.png`
- `artifacts/design/batcave-ui-overhaul/selected-explore-compact-light.png`

## Native implementation evidence

- `docs/assets/ui-overhaul/overview-cave-light.png`
- `docs/assets/ui-overhaul/explore-cave-light.png`
- `docs/assets/ui-overhaul/compact-drawer-cave-light.png`
- `docs/assets/ui-overhaul/chart-motion.gif`
- `artifacts/design/batcave-ui-overhaul/native-overview-compact-cave-light.png`
- `artifacts/design/batcave-ui-overhaul/native-explore-compact-cave-light.png`

## Comparison history

1. The first native render exposed the legacy stylesheet overriding the new shell and stretching Overview cards. The client-overhaul rules were moved after the legacy rules and the Overview grid was changed to content-sized rows.
2. Native WebKit rejected the icon library's generated barrel export. Direct component imports restored the same icon set and removed the native-only parse failure.
3. Cave light Overview and Explore captures were compared with the selected designs in `qa-overview-source-vs-native.png` and `qa-explore-source-vs-native.png`.
4. The implementation keeps the selected design's navigation, light colors, workload density, two-column Explore layout, and compact drawer.
5. CPU uses the existing uPlot chart in place of the radial mockup, avoiding another chart dependency. Contributor explanations use host-written copy. Models may select an eligible explanation ID but cannot write telemetry claims.
6. Native compact renders remain contained and scroll correctly. The workload drawer traps focus, closes with Escape, and restores logical focus. Automated checks also cover 200% text, reduced motion, stale and degraded states, and all family/mode pairs.

## Result

The recorded design comparison passed. These captures document that UI revision; later visible changes require fresh native checks.
