test("500 is the expected HTTP response", () => {
  expect(response.status).toBe(500);
  expect(response.header("content-type")).toContain("application/json");
});
