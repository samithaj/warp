# Plan: don't start a restored tab's shell until it is opened

**Status:** v2 — discovery + contracts. **Not ready to implement**; see §8 for what still has to be
decided first. v1 was rejected in review and its central architectural premise was wrong (§7).
**Related:** [`../resume-project-task/plan.md`](../resume-project-task/plan.md) (the project rail
that made this visible), [#9416](https://github.com/warpdotdev/warp/issues/9416) (session/PTY
ownership, adjacent).

---

## 1. The problem, observed

Restoring a window with ~50 tabs spawns ~50 shells at once. Symptoms, all one cause:

- Startup is visibly laggy; panes show *"Seems like your shell is taking a while to start…"*.
- Work is wasted: most restored tabs are never touched, yet each pays a full shell startup — with a
  heavy `zsh`/`starship`/`pyenv` profile that is seconds of CPU each.

A second symptom — the project rail filing every tab under **Other** and then re-sorting as shells
reported in — is **already fixed** (`4354e20ca`): a pane retains the directory it was restored with,
so `PaneGroup::session_path` can answer before the shell does. That fix is also a **prerequisite**
here: with deferral a tab has no live shell at all, so without the persisted directory every
untouched tab would sit in `Other` permanently.

## 2. Goal

A restored tab does not spawn its shell until the user opens it. The rail, the tab bar, and session
restoration behave exactly as they do now — including across a second save/restore cycle.

## 3. Non-goals

- Keeping shells *alive* across relaunch ([#9416](https://github.com/warpdotdev/warp/issues/9416)).
- Changing behaviour for newly-created tabs.
- Lazy-loading block history. Restored blocks already render from persistence; only the PTY defers.

---

## 4. Where the manager actually lives (corrected)

`TerminalPane` does **not** own the terminal manager. Its fields are `model_event_sender`, `uuid`,
`startup_directory`, `pane_configuration`, and `view: ViewHandle<TerminalPaneView>`
(`app/src/pane_group/pane/terminal_pane.rs:79-101`).

The manager is **`PaneStack` associated data for the backing pane view**, passed into `PaneView::new`
(`:150`). The comment inside `TerminalPane` that mentions declaring `terminal_manager` before `view`
describes that ownership relationship — it is not a field. Any design that "adds a state enum to
`TerminalPane`" is therefore addressing the wrong object.

**Consequence:** the deferred state has to be expressed in the pane-view/`PaneStack` associated-data
model, either as an optional manager slot or as a distinct deferred backing view that is swapped for
a live one on first start — while preserving the real drop relationship (manager torn down before
the view, so the event loop halts before cleanup).

## 5. There is no "surface without PTY" seam today

`create_session` (`app/src/pane_group/mod.rs:5988`) hands restored blocks and restoration state
straight into `LocalTtyTerminalManager::create_model` (`:6043`), which returns the surface **and**
the manager together. Remote and mock managers follow the same shape (`:6010`, `:6076`).

So constructing a normal-looking restored surface *without* scheduling shell startup does not exist
as a code path. Introducing that split — and making the later attach idempotent — is the core of the
work, not a mechanical follow-on. v1's claim that everything after a survey is "mechanical" was
wrong.

## 6. Snapshotting a pane that never started (blocker)

`TerminalPane::snapshot` (`:471`) derives **every** field from the live view:

| Snapshot field | Source |
|---|---|
| `cwd` | `view.pwd_if_local(app)` |
| `is_read_only` | `view.model.lock().is_read_only()` |
| `shell_launch_data` | `view.shell_launch_data_if_local(app)` |
| `input_config` | `view.input_config(...)` |
| `is_active`, `llm_model_override`, `active_profile_id`, conversation ids | live view / models |

A deferred pane has no live view to ask. Saving one would therefore write a **degraded** snapshot,
and a restore → quit → restore cycle would silently lose shell choice, cwd, input config, profile,
or conversation metadata — permanently, since each cycle re-saves the loss.

**Required contract:** a deferred pane re-snapshots **losslessly**.

**Status: implemented for the fields that can already be lost today.** `TerminalPane` now retains
the `TerminalPaneSnapshot` it was restored from, and `snapshot()` falls back per field through
`preserved_on_save` for `cwd` and `shell_launch_data` — the two whose live answers only exist once
the shell has reported.

This was not hypothetical: with many tabs restoring, quitting during the startup window already
persisted `None` for both, so the pane reopened in the default directory with the default shell and
saved that loss as the new truth. Fixed independently of deferral, and it is the same machinery
deferral needs.

Remaining for deferral: the other snapshot fields (`is_read_only`, `input_config`,
`llm_model_override`, `active_profile_id`, conversation ids) come from models that exist without a
shell, so they are safe today — but a pane with **no view at all** would need the same treatment.
Decide when the deferred representation exists (§4).

## 7. What was wrong in v1

Recorded so the error isn't repeated:

1. **False premise.** v1 asserted `TerminalPane` holds a non-optional manager and proposed putting a
   `SessionState` enum there. It holds no manager (§4). A doc comment was misread as a field.
2. **"Activation as the only trigger" is not a safe first step.** Restoration builds every tab and
   then activates the saved one through the workspace path (`app/src/workspace/view.rs:4007`,
   `:5408`), and other operations address background panes directly — notably synchronized/broadcast
   input. Deferral cannot ship with triggers unresolved.
3. **Snapshot losslessness was filed as "open question 5"**, i.e. optional. It is a blocker (§6).
4. **"Mechanical after the survey"** understated the lifecycle split (§5).

## 8. Contracts that must be decided before implementation

Each needs a written answer, not a TODO:

1. **Exactly-once start of the initially active pane.** Restoration activates the saved active tab;
   that path must start exactly one shell, and must not double-start if activation fires twice.
2. **Per-operation behaviour against a deferred pane** — for each of: pane focus, tab activation,
   text input, synchronized/broadcast input, split, agent resume, "run command in tab", search
   across tabs. Each either **starts** the pane, **rejects**, or **queues**. Broadcast input is the
   sharp case: it addresses background panes by design, so either it starts every deferred pane
   (defeating the feature) or it must skip/queue them, which is a visible behaviour change.
3. **Input contract.** Queued, start-on-input, or unsupported. Queuing implies buffering before a
   PTY exists.
4. **Lossless re-snapshot** (§6).
5. **Close before start.** Closing an untouched deferred pane must never spawn it, and must still
   clean up persisted block lists correctly.
6. **Partial-start failure.** If startup fails midway (shell missing, directory gone), what state is
   the pane left in, and is it retryable?
7. **Opt-in or default?** A setting, or automatic above N restored tabs. Default-on is a behaviour
   change to session restoration.
8. **Visual state.** What a deferred tab shows before it starts — restored blocks with no prompt,
   presumably — so it isn't mistaken for a hung shell.

## 9. Acceptance tests (deterministic, required)

- Inactive restored panes spawn **zero** PTYs.
- The initially active restored pane spawns **exactly one**.
- First activation is **idempotent** — activating twice starts once.
- Background/broadcast input follows the contract chosen in §8.2, asserted explicitly.
- Closing an untouched pane **never** spawns it.
- **Second save/restore preserves every snapshot field** for a pane that was never started (§6).

## 10. Risks

| Risk | Note |
|---|---|
| Silent snapshot degradation | §6. The worst failure mode: invisible, and compounds each cycle |
| Broadcast/synchronized input | §8.2. Either defeats the feature or changes behaviour |
| Drop ordering | Manager must still tear down before the view; the relationship is in `PaneStack`, not `TerminalPane` |
| Liveness assumptions | Every caller reaching a manager through a pane is a potential assumption; needs enumeration |
| Cloud/shared-session/ambient panes | Already have no local shell and pass `None` for startup directory; must stay unchanged |

## 11. Sequencing

1. **Enumerate** every caller that reaches a `TerminalManager` through a pane, and classify each as
   fine-without / must-start / requires-live. Output is a list and a scope decision.
2. **Answer §8** in writing. Steps 1 and 2 gate everything else.
3. Introduce the deferred representation in the pane-view/`PaneStack` model with **no behaviour
   change** (always starts eagerly; the state simply exists).
4. Add the lossless snapshot path (§6) and its test — before any deferral is enabled.
5. Flip restoration to deferred behind a feature flag, with the §8.2 triggers implemented.
6. Measure startup time and CPU with ~50 restored tabs, before and after.

Steps 1–2 are the real decision point. This spec deliberately does **not** propose a state machine
until they are done, because v1 shows what happens when the design is drawn before the ownership
model is confirmed.
