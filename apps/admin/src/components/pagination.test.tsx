import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { Pagination } from "@/components/pagination";

describe("Pagination", () => {
  it("marks the current page and moves between pages", async () => {
    const onChange = vi.fn();
    const user = userEvent.setup();
    render(<Pagination page={2} pages={5} onChange={onChange} />);

    expect(screen.getByRole("button", { name: "2" })).toHaveAttribute("aria-current", "page");

    await user.click(screen.getByRole("button", { name: "3" }));
    expect(onChange).toHaveBeenCalledWith(3);

    await user.click(screen.getByRole("button", { name: "Next page" }));
    expect(onChange).toHaveBeenCalledWith(3);

    await user.click(screen.getByRole("button", { name: "Previous page" }));
    expect(onChange).toHaveBeenCalledWith(1);
  });

  it("renders nothing for a single page", () => {
    const { container } = render(<Pagination page={1} pages={1} onChange={() => {}} />);
    expect(container).toBeEmptyDOMElement();
  });

  it("collapses long page ranges with a gap", () => {
    render(<Pagination page={50} pages={117} onChange={() => {}} />);
    expect(screen.getAllByText("…")).toHaveLength(2);
    expect(screen.getByRole("button", { name: "1" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "117" })).toBeInTheDocument();
  });
});
