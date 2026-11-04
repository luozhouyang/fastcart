import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { OrderPill, PaymentPill, StockPill } from "@/components/status-pill";

describe("status pills", () => {
  it("labels order and payment states", () => {
    const { rerender } = render(<OrderPill status="processing" />);
    expect(screen.getByText("Processing")).toBeInTheDocument();

    rerender(<OrderPill status="canceled" />);
    expect(screen.getByText("Canceled")).toBeInTheDocument();

    rerender(<PaymentPill status="partially_refunded" />);
    expect(screen.getByText("Partial refund")).toBeInTheDocument();
  });

  it("classifies stock health", () => {
    const { rerender } = render(<StockPill available={0} />);
    expect(screen.getByText("Out of stock")).toBeInTheDocument();

    rerender(<StockPill available={6} />);
    expect(screen.getByText("Low stock")).toBeInTheDocument();

    rerender(<StockPill available={42} />);
    expect(screen.getByText("In stock")).toBeInTheDocument();
  });
});
