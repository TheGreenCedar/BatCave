# Live layout and inspection

Explore gives the workload list its full available width. Selecting a workload or choosing **Inspect resource** opens a modal side drawer at every window width. **Close**, Escape, and a click on the backdrop dismiss it. Resizing keeps the drawer, selected identity, history, and focused detail control. Dismissal restores its opener when that control still exists; otherwise it restores the selected workload or resource control, then Explore navigation as a fallback.

Closing the drawer stops workload inspection requests without clearing the selected identity or recorded history. Reopening requests current evidence. Existing asynchronous response admission and memory-credit cleanup continue to apply.

Overview keeps monitoring status separate from metric cards. Status and contributor copy have bounded, keyboard-focusable scroll regions; long text remains available at 200% text size. Resource cards reserve their quality-label line so a quality change does not add a row. Read/write and download/upload readings retain complete values and units. Unavailable workload readings use a dash and expose the reported quality reason to assistive technology; measured zero retains its numeric value.

From `src/BatCave.App`, run:

```powershell
npm run verify
$env:BATCAVE_ACCESSIBILITY_TEST_PORT = '1434'
npm run test:accessibility
```

The browser suite checks drawer dismissal and focus, resizing, full-width list geometry, long status text, 200% text, quality reasons, history, and existing rate formatting. Browser fixture checks and screenshots prove layout only. Fresh native Windows acceptance must bind screenshots and interactions to the actual source and installed app identity before reporting the product behavior as verified. See [design QA](../design-qa.md).
