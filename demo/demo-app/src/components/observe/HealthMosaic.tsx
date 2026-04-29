import Mosaic, { MosaicCell } from '../Mosaic';
import type { HealthData } from './StatusTab';

export interface HealthMosaicProps {
  health: HealthData;
}

function fmtUptime(secs: number): string {
  if (secs < 60) return `${secs.toFixed(0)}s`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m`;
  return `${Math.floor(secs / 3600)}h ${Math.floor((secs % 3600) / 60)}m`;
}

/**
 * Top-level health mosaic: agents, plans, runs, uptime.
 * Extracted from the Explorer header pills.
 */
export function HealthMosaic({ health }: HealthMosaicProps) {
  return (
    <Mosaic columns={4}>
      <MosaicCell
        label="Agents"
        value={String(health.active_agents ?? 0)}
        color="dream"
        mono
      />
      <MosaicCell
        label="Plans"
        value={String(health.active_plans ?? 0)}
        color="bone"
        mono
      />
      <MosaicCell
        label="Runs"
        value={String(health.active_runs ?? 0)}
        color="rose"
        mono
      />
      <MosaicCell
        label="Uptime"
        value={fmtUptime(health.uptime_secs ?? 0)}
        color="success"
      />
    </Mosaic>
  );
}
