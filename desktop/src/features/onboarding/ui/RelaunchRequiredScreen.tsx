import { RecoveryScreen } from "./RecoveryScreen";

export function RelaunchRequiredScreen() {
  return (
    <RecoveryScreen
      testId="relaunch-required"
      title="Restart SchoolX to finish recovery"
      body="Your identity was updated. SchoolX needs to restart so syncing and agents run under it."
    />
  );
}
