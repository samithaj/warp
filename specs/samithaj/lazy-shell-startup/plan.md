# Plan: don't start a restored tab's shell until it is opened

**Status:** spec — not started. Written after observing the problem in a real build.
**Related:** [`../resume-project-task/plan.md`](../resume-project-task/plan.md) (the project rail
that made this visible), [#9416](https://github.com/warpdotdev/warp/issues/9416) (session/PTY
ownership, adjacent).

---

## 1. The problem, observed

Restoring a window with ~50 tabs spawns ~50 shells at once. Symptoms, all from one cause:

- Startup is visibly laggy; panes show *"Seems like your shell is taking a while to start…"*.
- The project rail churns: every tab appears under **Other**, then re-sorts into its real project
  over several seconds as each shell reports a directory. It reads as the rail grouping things late.
- Work is wasted: most restored tabs are never touched in a session, yet each pays a full shell
  startup — for a user with a heavy `zsh`/`starship`/`pyenv` profile that is seconds of CPU each.

The rail-churn half is **already fixed** (`4354e20ca`): a pane now retains the startup directory it
was restored with, so `PaneGroup::session_path` can answer before the shell does. That fix is also a
**prerequisite for this one** — with lazy startup a tab has no live shell at all, so without the
persisted directory every untouched tab would sit in `Other` permanently.

What remains is the actual cost: spawning PTYs nobody asked for.

## 2. Goal

A restored tab does not spawn its shell until the user opens it. The rail, the tab bar, and session
restoration behave exactly as they do now.

## 3. Non-goals

- Keeping shells *alive* across relaunch ([#9416](https://github.com/warpdotdev/warp/issues/9416)) —
  the opposite problem, and a much larger one.
- Changing behaviour for newly-created tabs; a tab the user just opened should start immediately.
- Lazy-loading block history. Restored blocks already render without a live shell; only the PTY is
  deferred.

---

## 4. What makes this non-trivial

`PaneGroup::create_session` builds the `TerminalView` **and** the `TerminalManager` together
(`app/src/pane_group/mod.rs:5956`), and the restore path calls it for every leaf
(`:1671`). `TerminalPane` then holds `terminal_manager` as a non-optional field, with a comment
noting the declaration order matters for drop ordering:

```rust
/// Defining `terminal_manager` before `view` means that `terminal_manager`
/// gets dropped first (guaranteed by the language), which halts the event
/// loop and avoids possible deadlocks during session cleanup.
```

So "no shell yet" is not currently representable. The work is introducing that state and auditing
who assumes it cannot happen.

**Survey first.** Before designing, count the callers that reach a `TerminalManager` from a pane and
classify each as (a) fine with "not started yet", (b) needs to trigger startup, (c) genuinely
requires a running shell. That survey determines whether this is a week or a month.

---

## 5. Sketch of an approach

Model the pane's session as a state rather than a value:

```
SessionState::Deferred { startup_directory, shell_launch_data, … }   // restored, not spawned
SessionState::Live(ModelHandle<Box<dyn TerminalManager>>)
```

- **Deferred → Live** on first activation, plus anything that genuinely needs a shell (running a
  command, resuming an agent task, splitting the pane).
- The restored **block list still renders** in the deferred state — that is what makes the tab look
  normal, and it already comes from persistence rather than the PTY.
- Keep drop-order safety: the manager stays declared before the view; `Deferred` simply has none.

### Where the directory comes from

Already solved. `TerminalPane::startup_directory` (`4354e20ca`) holds the restored cwd, and
`session_path` falls back to it, so project bucketing works with zero shells running.

---

## 6. Risks

| Risk | Note |
|---|---|
| Wide blast radius | Every `terminal_manager` caller is a potential assumption of liveness. The survey in §4 is the gate on scoping this at all. |
| Silent behaviour change | A tab that used to have a live shell now may not. Anything reading shell state (env vars, cwd tracking, shell integration) needs an explicit answer. |
| Agent sessions | A restored tab that had a CLI agent must not appear "running". It does not today either — the agent is gone — but the deferred state must not be confused with it. |
| Session sharing / cloud panes | These have no local shell already and must keep working unchanged; they pass `None` for the startup directory today. |
| Drop ordering | The existing comment on `TerminalPane` is a real hazard, not decoration. Preserve it. |

---

## 7. Open questions

1. **What triggers startup besides activation?** Command palette "run in tab", agent resume,
   split-pane, search across tabs, `broadcast to all panes` — each needs a decision.
2. **Does anything enumerate live shells** (session sharing, telemetry, the terminal server) and
   would a deferred pane break its accounting?
3. **Is deferral opt-in?** A setting, or on by default above N restored tabs? Defaulting on for
   everyone is a behaviour change to session restoration.
4. **What does the tab look like before it starts?** Restored blocks, presumably, with no prompt —
   needs a design answer so it is not mistaken for a hung shell.
5. **Interaction with `restore_session`** — does the snapshot need to record that a pane was never
   started, or is that purely runtime state?

---

## 8. Suggested sequencing

1. **Survey** (§4) — enumerate and classify every `terminal_manager` reach-through. Output is a list,
   and a go/no-go on scope.
2. Introduce the deferred state with **no behaviour change** (everything starts eagerly, the state
   simply exists and is always `Live`).
3. Flip restoration to `Deferred` behind a feature flag, with activation as the only trigger.
4. Add the remaining triggers found in step 1.
5. Measure: startup time and CPU with ~50 restored tabs, before and after.

Step 1 is the real decision point. Everything after it is mechanical but broad.
