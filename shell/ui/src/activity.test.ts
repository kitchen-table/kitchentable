import {
  ACTION,
  type AccessEvent,
  ago,
  describe as describeAction,
  filterAvailable,
  matches,
  meta,
  sentence,
  when,
} from "./activity";

function event(over: Partial<AccessEvent> = {}): AccessEvent {
  return { at: 1_700_000_000, actor: "viewer", action: "opened", ...over };
}

/**
 * Every action the daemon writes, gathered in one place.
 *
 * The list is `kt-server`'s `refused`, `kt-daemon`'s `requested` and `opened`,
 * the four the rpc layer writes for device decisions, and `forgotten`. An
 * action missing from [`ACTION`] renders as a raw lowercase verb beside a
 * meaningless dot, which is how `refused` - the most common negative event in
 * the product - shipped unlabelled. So this asserts the whole set rather than a
 * sample: add a row here whenever the daemon learns to write one.
 */
const WRITTEN = [
  "opened",
  "requested",
  "paired",
  "denied",
  "revoked",
  "refused",
  "forgotten",
];

describe("activity", () => {
  it("has wording and a glyph for every action the daemon writes", () => {
    for (const action of WRITTEN) {
      expect(ACTION[action], `${action} has no entry`).toBeDefined();
    }
  });

  it("falls back rather than throwing on an action it has never heard of", () => {
    const unknown = describeAction("teleported");
    expect(unknown.title).toBe("teleported");
    expect(unknown.glyph).toBe("·");
  });

  describe("when", () => {
    it("says 'ago' only where it reads correctly", () => {
      const now = 1_700_000_000;
      expect(when(now - 600, now)).toBe("10m ago");
      expect(when(now - 7200, now)).toBe("2h ago");
      // "just now ago" and "12 Aug ago" are both wrong, so neither takes it.
      expect(when(now - 5, now)).toBe("just now");
      expect(when(now - 60 * 86_400, now)).toBe(ago(now - 60 * 86_400, now));
      expect(when(now - 60 * 86_400, now)).not.toMatch(/ago/);
    });
  });

  describe("meta", () => {
    it("names the device by the fingerprint the Devices tab shows", () => {
      // Somebody comparing a row here against a row there needs the same
      // string in both places, and the id is neither short nor meaningful.
      expect(
        meta(event(), { fingerprint: "a7:2f:9c", agent: "Safari · iPhone" }),
      ).toBe("device · a7:2f:9c · Safari · iPhone");
    });

    it("says who without a device, and carries any detail", () => {
      expect(meta(event({ actor: "owner", detail: "from the CLI" }))).toBe(
        "you · from the CLI",
      );
      expect(meta(event({ actor: "system" }))).toBe("kitchen table");
    });

    it("shows an actor it does not recognise rather than dropping it", () => {
      expect(meta(event({ actor: "somebody-new" }))).toBe("somebody-new");
    });
  });

  describe("sentence", () => {
    it("names the device when we have one, and stays true when we do not", () => {
      expect(sentence(event(), "Kitchen iPad")).toBe("Kitchen iPad opened the app");
      expect(sentence(event())).toBe("Someone opened the app");
      expect(sentence(event({ actor: "owner" }))).toBe("You opened the app");
    });

    it("stops saying the app twice once the row has already named it", () => {
      // "Chores Rota · Someone opened the app" says it at both ends.
      expect(sentence(event(), undefined, { appNamed: true })).toBe(
        "Someone opened it",
      );
      expect(sentence(event(), "Kitchen iPad", { appNamed: true })).toBe(
        "Kitchen iPad opened it",
      );
    });

    it("leaves the wording alone where the app was never in it", () => {
      // Only actions whose phrase mentions the app need a second form; pairing
      // a device is about the device, wherever the row is drawn.
      const paired = event({ action: "paired" });
      expect(sentence(paired, undefined, { appNamed: true })).toBe(
        sentence(paired),
      );
    });
  });

  describe("filters", () => {
    it("puts every device decision under Devices, including the refusals", () => {
      // A revocation and the refusals that follow it are the same story, and
      // filtering to the owner's own decisions would show the cause and hide
      // the effect.
      for (const action of ["requested", "paired", "denied", "revoked", "refused", "forgotten"]) {
        expect(matches(event({ action }), "devices"), action).toBe(true);
      }
      expect(matches(event({ action: "opened" }), "devices")).toBe(false);
    });

    it("keeps All meaning all", () => {
      for (const action of WRITTEN) {
        expect(matches(event({ action }), "all"), action).toBe(true);
      }
    });

    it("marks Deploys unavailable, because nothing writes one", () => {
      // The daemon has no `deployed` action until versioning lands, so this
      // chip could only ever empty the list. Delete this expectation - and the
      // list it reads - the moment something writes a deploy.
      expect(filterAvailable("deploys")).toBe(false);
      expect(WRITTEN).not.toContain("deployed");
      for (const filter of ["all", "opens", "devices"] as const) {
        expect(filterAvailable(filter), filter).toBe(true);
      }
    });
  });
});
