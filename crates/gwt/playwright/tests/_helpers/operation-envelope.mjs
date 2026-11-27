import { setTimeout as delay } from "node:timers/promises";

// A refused operation carries `error` and no `output`. Parsing the missing
// output hid the refusal behind `"undefined" is not valid JSON` (Issue #7752).
export function operationOutput(operation, envelope) {
  if (envelope?.ok !== true || envelope.output === undefined) {
    throw new Error(`Fixture ${operation} failed: ${JSON.stringify(envelope ?? null)}`);
  }
  return typeof envelope.output === "string" ? JSON.parse(envelope.output) : envelope.output;
}

// Only for reads: a busy daemon may refuse one, and repeating it changes nothing.
// Every refusal is reported, and the last one fails the caller once the bound is spent.
export async function readOperationOutput(operation, run, { attempts = 5, wait = () => delay(2_000), onRefusal = () => {} } = {}) {
  for (let attempt = 1; ; attempt++) {
    try {
      return operationOutput(operation, await run());
    } catch (error) {
      onRefusal({ operation, attempt, error: String(error?.message ?? error) });
      if (attempt >= attempts) throw error;
      await wait();
    }
  }
}
