// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import { IERC20 } from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import { RoleRegistry } from "./RoleRegistry.sol";

/// @title ISFRBountyPool — reward pool for ISFR keeper submissions.
/// @notice The oracle (ORACLE_ROLE) calls `rewardKeeper()` after each valid
///         rate submission. The pool pays out a fixed `rewardPerSubmission`
///         from its token balance.
contract ISFRBountyPool {
    bytes32 public constant ORACLE_ROLE = keccak256("ORACLE_ROLE");

    RoleRegistry public immutable roleRegistry;
    IERC20 public immutable token;

    /// @notice Reward per valid submission (in token units).
    uint256 public rewardPerSubmission;

    /// @notice Total rewards paid out.
    uint256 public totalPaid;

    event KeeperRewarded(address indexed keeper, uint256 amount);
    event RewardUpdated(uint256 oldReward, uint256 newReward);

    error NotOracle();
    error InsufficientBalance();

    constructor(address roleRegistry_, address token_, uint256 rewardPerSubmission_) {
        roleRegistry = RoleRegistry(roleRegistry_);
        token = IERC20(token_);
        rewardPerSubmission = rewardPerSubmission_;
    }

    modifier onlyOracle() {
        if (!roleRegistry.hasRole(ORACLE_ROLE, msg.sender)) revert NotOracle();
        _;
    }

    /// @notice Reward a keeper for a valid submission. Called by the oracle.
    function rewardKeeper(address keeper) external onlyOracle {
        uint256 bal = token.balanceOf(address(this));
        if (bal < rewardPerSubmission) revert InsufficientBalance();
        bool ok = token.transfer(keeper, rewardPerSubmission);
        require(ok, "transfer failed");
        totalPaid += rewardPerSubmission;
        emit KeeperRewarded(keeper, rewardPerSubmission);
    }

    /// @notice Pool balance available for rewards.
    function availableBalance() external view returns (uint256) {
        return token.balanceOf(address(this));
    }
}
