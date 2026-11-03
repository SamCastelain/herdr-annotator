"""Generate a private tabs mod without changing Herdr's shipped mod."""

def integrated_tabs(source):
    anchor = '.when(enabled.includes("changes"), el => el.child(panelButton("changes", "Changes", "mods/changes/changes.svg")))'
    if source.count(anchor) != 1:
        raise ValueError("Unsupported shipped tabs mod: Changes button changed; no files were installed")
    return 'import { openReview } from "./launcher.js";\n' + source.replace(anchor, '''.when(enabled.includes("changes"), el => el.child(h_flex().gap(0).items_center()
        .child(panelButton("changes", "Changes", "mods/changes/changes.svg"))
        .child(div().id("panel-review-full").h(26).px(8).flex_none().flex().items_center().rounded(6)
          .border_l(1).border_color(t.border).text_size(12).text_color(this.reviewError ? t.ansi1 : t.muted)
          .hover(s => s.bg(t.hover).text_color(t.foreground)).on_mouse_down("left", stop)
          .on_click((_e, cx) => openReview(this, cx))
          .child(this.openingReview ? "…" : this.reviewError ? `Retry ↗ · ${this.reviewError}` : "↗"))))''')
