/** A zero process exit is insufficient: the driver must explicitly finish and
 * report no error. A wrapper-initiated termination after success is accepted. */
export function unityDriverExitCode(result) {
  if (result.finishedOk === true && !result.driverError) return 0;
  return typeof result.code === "number" && result.code !== 0 ? result.code : 1;
}
