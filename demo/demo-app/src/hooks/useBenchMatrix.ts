/**
 * useBenchMatrix — re-exports useMatrixBench for consistent naming.
 * The underlying useMatrixBench hook manages matrix configuration and
 * multi-model evaluation progress. This file exists so that the split
 * sub-hook set (useBenchRuns, useBenchFilter, useBenchMatrix) uses a
 * consistent naming convention.
 */
export { useMatrixBench as useBenchMatrix } from './useMatrixBench';
