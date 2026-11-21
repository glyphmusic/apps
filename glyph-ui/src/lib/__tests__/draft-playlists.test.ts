import { beforeEach, describe, expect, it } from "vitest";
import { app } from "../state.svelte";
import { createDraftPlaylists, draftIdOf, draftValue, isDraftValue } from "../draft-playlists.svelte";
import { back, mg, migrateDrafts, promptNewTarget, promptRenameTarget, setTarget, toPick } from "../settings/migrate-state.svelte";

describe("draft playlists", () => {
  beforeEach(() => {
    app.nameDialog = null;
    migrateDrafts.clear();
    mg.targets = {};
    mg.picked = {};
    mg.step = "choose";
  });

  it("encodes and decodes draft target values", () => {
    const v = draftValue("draft-1");
    expect(isDraftValue(v)).toBe(true);
    expect(draftIdOf(v)).toBe("draft-1");
    expect(isDraftValue("pl:abc")).toBe(false);
    expect(draftIdOf("pl:abc")).toBeNull();
  });

  it("prompts via NameDialog and only creates on submit", () => {
    const drafts = createDraftPlaylists();
    let createdId: string | null = null;
    drafts.promptCreate({
      initial: "Roadtrip",
      onCreated: (d) => {
        createdId = d.id;
      },
    });
    expect(app.nameDialog).not.toBeNull();
    expect(app.nameDialog?.initial).toBe("Roadtrip");
    expect(drafts.items).toHaveLength(0);

    // Simulating modal submit creates the draft
    app.nameDialog?.onSubmit?.("Summer Mix");
    expect(createdId).not.toBeNull();
    expect(drafts.items).toHaveLength(1);
    expect(drafts.items[0].name).toBe("Summer Mix");
  });

  it("renames a draft via NameDialog", () => {
    const drafts = createDraftPlaylists();
    const d = drafts.create("Old Name");
    drafts.promptRename(d.id);
    expect(app.nameDialog?.initial).toBe("Old Name");
    expect(app.nameDialog?.nonEmpty).toBe(true);
    app.nameDialog?.onSubmit?.("New Name");
    expect(drafts.get(d.id)?.name).toBe("New Name");
  });

  it("prunes unused drafts automatically", () => {
    const drafts = createDraftPlaylists();
    const d1 = drafts.create("First");
    const d2 = drafts.create("Second");
    expect(drafts.items).toHaveLength(2);
    drafts.prune([d2.id]);
    expect(drafts.items.map((d) => d.id)).toEqual([d2.id]);
    expect(drafts.get(d1.id)).toBeUndefined();
  });

  it("deletes unused migrate drafts when switching targets or leaving choose", () => {
    mg.targets = { c1: "liked", c2: "liked" };
    promptNewTarget("c1", "");
    expect(app.nameDialog).not.toBeNull();
    app.nameDialog?.onSubmit?.("Gym Mix");

    expect(migrateDrafts.items).toHaveLength(1);
    const d1 = migrateDrafts.items[0];
    expect(mg.targets.c1).toBe(draftValue(d1.id));

    // Share the same draft with c2
    setTarget("c2", draftValue(d1.id));
    expect(migrateDrafts.items).toHaveLength(1);

    // Switching c1 away keeps d1 because c2 still uses it
    setTarget("c1", "liked");
    expect(migrateDrafts.items).toHaveLength(1);

    // Renaming via promptRenameTarget updates the existing draft in place
    promptRenameTarget("c2");
    app.nameDialog?.onSubmit?.("Cardio Mix");
    expect(migrateDrafts.get(d1.id)?.name).toBe("Cardio Mix");

    // Switching c2 away leaves d1 unused -> auto-deleted
    setTarget("c2", "pl:existing");
    expect(migrateDrafts.items).toHaveLength(0);

    // Creating another draft then navigating away clears it
    promptNewTarget("c1", "");
    app.nameDialog?.onSubmit?.("Temp");
    expect(migrateDrafts.items).toHaveLength(1);
    back();
    expect(migrateDrafts.items).toHaveLength(0);

    // Same for toPick()
    mg.step = "choose";
    promptNewTarget("c1", "");
    app.nameDialog?.onSubmit?.("Temp 2");
    expect(migrateDrafts.items).toHaveLength(1);
    toPick();
    expect(migrateDrafts.items).toHaveLength(0);
  });
});
