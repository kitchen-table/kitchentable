import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { vi } from "vitest";
import { NewAppModal, slugify } from "./NewAppModal";

function show(over: Partial<Parameters<typeof NewAppModal>[0]> = {}) {
  const onCreate = vi.fn().mockResolvedValue({ slug: "packing-list" });
  const onPick = vi.fn().mockResolvedValue({ slug: "trip-planner" });
  const onClose = vi.fn();
  const view = render(
    <NewAppModal
      workspace="/ws"
      onCreate={onCreate}
      onPick={onPick}
      onClose={onClose}
      {...over}
    />,
  );
  return { ...view, onCreate, onPick, onClose };
}

describe("NewAppModal", () => {
  it("creates an app from a typed name", async () => {
    const { onCreate, onClose } = show();

    fireEvent.change(screen.getByPlaceholderText("Packing list"), {
      target: { value: "Packing List" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Add app" }));

    await waitFor(() => expect(onCreate).toHaveBeenCalledWith("Packing List"));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it("submits on Enter, so the keyboard is enough", async () => {
    const { onCreate } = show();

    const field = screen.getByPlaceholderText("Packing list");
    fireEvent.change(field, { target: { value: "Notes" } });
    fireEvent.keyDown(field, { key: "Enter" });

    await waitFor(() => expect(onCreate).toHaveBeenCalledWith("Notes"));
  });

  it("will not create an app with no usable name", () => {
    show();
    expect(screen.getByRole("button", { name: "Add app" }).hasAttribute("disabled")).toBe(true);

    fireEvent.change(screen.getByPlaceholderText("Packing list"), {
      target: { value: "   " },
    });
    expect(screen.getByRole("button", { name: "Add app" }).hasAttribute("disabled")).toBe(true);
  });

  it("previews the slug the daemon will pick", () => {
    show();
    fireEvent.change(screen.getByPlaceholderText("Packing list"), {
      target: { value: "Trip Planner!" },
    });
    expect(screen.getByText('"trip-planner"')).toBeDefined();
  });

  it("opens the folder picker from the drop zone", async () => {
    const { onPick, onClose } = show();

    fireEvent.click(screen.getByRole("button", { name: /Drop a folder here/ }));

    await waitFor(() => expect(onPick).toHaveBeenCalled());
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it("stays open when the picker is cancelled", async () => {
    // Resolving to null is a cancel, not a success to close on.
    const onPick = vi.fn().mockResolvedValue(null);
    const { onClose } = show({ onPick });

    fireEvent.click(screen.getByRole("button", { name: /Drop a folder here/ }));

    await waitFor(() => expect(onPick).toHaveBeenCalled());
    expect(onClose).not.toHaveBeenCalled();
  });

  it("shows the daemon's own words when it refuses", async () => {
    // Daemon errors are {code, message} rather than Errors, and the message is
    // already written for a person.
    const onCreate = vi
      .fn()
      .mockRejectedValue({ code: "bad_request", message: "an app needs a name" });
    const { onClose } = show({ onCreate });

    fireEvent.change(screen.getByPlaceholderText("Packing list"), {
      target: { value: "Notes" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Add app" }));

    await waitFor(() => expect(screen.getByRole("alert").textContent).toBe("an app needs a name"));
    expect(onClose).not.toHaveBeenCalled();
  });

  it("says what is happening while a dropped folder is copied", () => {
    show({ dropping: true });
    expect(screen.getByText("Copying it in…")).toBeDefined();
    // The primary action reads as busy too, so there is no button that looks
    // ready to press while a copy is in flight.
    expect(screen.getByRole("button", { name: "Adding…" }).hasAttribute("disabled")).toBe(true);
  });

  it("says the folder is copied, not moved", () => {
    // Someone is about to hand over a folder they care about.
    show();
    expect(screen.getByText(/copied into \/ws/)).toBeDefined();
  });
});

describe("slugify", () => {
  it("matches the shape the registry picks", () => {
    expect(slugify("Trip Planner")).toBe("trip-planner");
    expect(slugify("  Packing List!  ")).toBe("packing-list");
    expect(slugify("Notes 2024")).toBe("notes-2024");
    expect(slugify("...")).toBe("");
  });
});
