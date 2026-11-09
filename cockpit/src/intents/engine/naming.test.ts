import { expect, test } from "vitest";
import { centralPhrases, medoid, maskedTemplate, objectName } from "./naming.ts";
import { intentPhrases, namePhrases } from "./names-job.ts";

test("central phrases represent the repeated goal before an unrelated outlier", () => {
  const members = ["refund an order", "check the CI tests", "check CI tests", "check the CI tests"];
  expect(centralPhrases(members, 2)).toEqual(["check CI tests", "check the CI tests"]);
  expect(medoid(members)).toBe("check the CI tests");
  expect(namePhrases([...members, ...members])).toEqual(["refund an order", "check the CI tests", "check CI tests"]);
  expect(intentPhrases(["a", "missing", "b"], new Map([["a", members.slice(0, 2)], ["b", members.slice(2)]]))).toEqual(namePhrases(members));
});

test("rule names mask record values and preserve the shared object", () => {
  expect(maskedTemplate(["assign this task to emp-1", "assign this task to emp-2"])).toBe("assign this task to {id}");
  expect(objectName(["task"], ["create task", "list tasks"], [])).toBe("Task operations");
  expect(objectName([], [], ["github__read_issue"])).toBe("Github operations");
});
