import { render, screen } from "@testing-library/react";

import App from "./App";

describe("App", () => {
  it("renders the TokenBuddy shell", () => {
    render(<App />);

    expect(
      screen.getByRole("heading", { name: "TokenBuddy" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "发送" })).toBeInTheDocument();
  });
});
