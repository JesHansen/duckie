test("returns the sent request", () => {
  expect(response.status).toBe(200);
  expect(response.json().method).toBe("GET");
  expect(response.json().query).toEqual([["message", "Hello, Duckie"]]);
});

test("preserves duplicate response headers", () => {
  expect(response.headers("x-duplicate")).toEqual(["first", "second"]);
});
