// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import { RoleRegistry } from "./RoleRegistry.sol";

/// @title ISFROracle — on-chain ISFR rate storage with epoch-keyed submissions.
/// @notice Keepers call `submitRate()` each epoch. Latest rate is always readable
///         via `currentRate()`. Historical rates are kept per epoch.
contract ISFROracle {
    bytes32 public constant KEEPER_ROLE = keccak256("KEEPER_ROLE");

    struct Rate {
        uint256 epochId;
        uint256 compositeBps;
        uint256 lendingBps;
        uint256 structuredBps;
        uint256 fundingBps;
        uint256 stakingBps;
        uint256 confidenceBps;
        uint64 timestamp;
        address submitter;
    }

    RoleRegistry public immutable roleRegistry;
    address public bountyPool;

    /// @notice The most recently submitted rate.
    Rate public currentRate;

    /// @notice Epoch ID => Rate.
    mapping(uint256 => Rate) public epochRates;

    /// @notice Total number of submitted epochs.
    uint256 public submissionCount;

    event RateSubmitted(
        uint256 indexed epochId,
        uint256 compositeBps,
        uint256 confidenceBps,
        address indexed submitter
    );
    event BountyPoolSet(address indexed pool);

    error NotKeeper();
    error EpochAlreadySubmitted();

    constructor(address roleRegistry_, address) {
        roleRegistry = RoleRegistry(roleRegistry_);
    }

    modifier onlyKeeper() {
        if (!roleRegistry.hasRole(KEEPER_ROLE, msg.sender)) revert NotKeeper();
        _;
    }

    function setBountyPool(address pool) external {
        bountyPool = pool;
        emit BountyPoolSet(pool);
    }

    /// @notice Submit a rate for the given epoch. Only callable by KEEPER_ROLE holders.
    function submitRate(
        uint256 epochId,
        uint256 compositeBps,
        uint256 lendingBps,
        uint256 structuredBps,
        uint256 fundingBps,
        uint256 stakingBps,
        uint256 confidenceBps
    ) external onlyKeeper {
        if (epochRates[epochId].timestamp != 0) revert EpochAlreadySubmitted();

        Rate memory r = Rate({
            epochId: epochId,
            compositeBps: compositeBps,
            lendingBps: lendingBps,
            structuredBps: structuredBps,
            fundingBps: fundingBps,
            stakingBps: stakingBps,
            confidenceBps: confidenceBps,
            timestamp: uint64(block.timestamp),
            submitter: msg.sender
        });

        epochRates[epochId] = r;
        currentRate = r;
        submissionCount++;

        emit RateSubmitted(epochId, compositeBps, confidenceBps, msg.sender);
    }

    /// @notice Get the composite rate in bps for the latest submission.
    function getCurrentRate() external view returns (uint256) {
        return currentRate.compositeBps;
    }
}
