/** ISFR rate computation helpers and contract interaction builders. */

// -- Contract addresses (Ethereum mainnet) --

export const AAVE_V3_POOL = '0x87870Bca3F3fD6335C3F4ce8392D69350B4fA4E2';
export const USDC = '0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48';
export const COMPOUND_V3_COMET = '0xc3d688B66703497DAA19211EEdff47f25384cdc3';
export const SUSDE = '0x9D39A5DE30e57443BfF2A8307A4256c8797A3497';
export const BEACON_DEPOSIT = '0x00000000219ab540356cBB839Cbe05303d7705Fa';

// -- ISFR weights --

export const ISFR_WEIGHTS = {
  LENDING: 0.60,
  STRUCTURED: 0.25,
  FUNDING: 0.10,
  STAKING: 0.05,
} as const;

export type ISFRClass = keyof typeof ISFR_WEIGHTS;

// -- Cast call builders --

const RPC = 'http://127.0.0.1:8545';

export function castCallAaveSupplyRate(): string {
  return `cast call ${AAVE_V3_POOL} "getReserveData(address)(uint256,uint128,uint128,uint128,uint128,uint128,uint40,uint16,address,address,address,address,uint128,uint128,uint128)" ${USDC} --rpc-url ${RPC} 2>/dev/null | head -5`;
}

export function castCallCompoundSupplyRate(): string {
  return `cast call ${COMPOUND_V3_COMET} "getSupplyRate(uint256)(uint64)" 0 --rpc-url ${RPC} 2>/dev/null`;
}

export function castCallSusdeAssets(): string {
  return `cast call ${SUSDE} "totalAssets()(uint256)" --rpc-url ${RPC} 2>/dev/null`;
}

export function castCallSusdeSupply(): string {
  return `cast call ${SUSDE} "totalSupply()(uint256)" --rpc-url ${RPC} 2>/dev/null`;
}

export function castCallBeaconBalance(): string {
  return `cast balance ${BEACON_DEPOSIT} --rpc-url ${RPC} 2>/dev/null`;
}

export function castBlockNumber(): string {
  return `cast block-number --rpc-url ${RPC} 2>/dev/null`;
}

// -- Rate formatting --

/** Format a rate in basis points with 2 decimal places */
export function formatBps(bps: number): string {
  return `${bps.toFixed(2)} bps`;
}

/** Format a rate as a percentage */
export function formatPct(bps: number): string {
  return `${(bps / 100).toFixed(2)}%`;
}
