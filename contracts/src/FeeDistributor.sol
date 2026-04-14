// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import { IERC20 } from "@openzeppelin/contracts/token/ERC20/IERC20.sol";

/// @title FeeDistributor — 40/30/20/10 bounty split for the knowledge-flywheel demo.
/// @notice Pulls `totalFee` via `transferFrom` from msg.sender and credits pull-payment
///         balances in `earningsOf`:
///             - validators: 40% (split evenly, remainder → validators[0])
///             - data provider: 30%
///             - agent (winning worker): 20%
///             - treasury: 10% (plus any bps rounding dust)
///         Recipients call `claim()` to withdraw. CEI pattern: state is zeroed before
///         token transfer, so a standard ERC20 cannot reenter. No ReentrancyGuard by
///         design — callers who integrate with non-standard tokens must wrap.
contract FeeDistributor {
    uint256 public constant BPS_DENOM = 10_000;
    uint256 public constant VALIDATOR_BPS = 4_000; // 40%
    uint256 public constant PROVIDER_BPS = 3_000;  // 30%
    uint256 public constant AGENT_BPS = 2_000;     // 20%
    uint256 public constant TREASURY_BPS = 1_000;  // 10%

    IERC20 public immutable token;
    address public immutable treasury;

    /// @notice Pull-payment balances. Anyone credited here can call `claim()`.
    mapping(address => uint256) public earningsOf;
    /// @notice Cumulative total of all fees ever routed through this contract.
    uint256 public totalDistributed;

    event FeesDistributed(
        uint256 indexed jobId,
        address indexed agent,
        address indexed provider,
        address[] validators,
        uint256 totalFee,
        uint256 agentShare,
        uint256 providerShare,
        uint256 validatorTotalShare,
        uint256 treasuryShare
    );
    event FeesClaimed(address indexed beneficiary, uint256 amount);

    error ZeroAmount();
    error ZeroAgent();
    error ZeroProvider();
    error EmptyValidators();
    error NothingToClaim();
    error TransferFailed();

    constructor(address token_, address treasury_) {
        if (token_ == address(0)) revert ZeroAmount();
        if (treasury_ == address(0)) revert ZeroAmount();
        token = IERC20(token_);
        treasury = treasury_;
    }

    /// @notice Pull `totalFee` from msg.sender (must be pre-approved) and credit
    ///         the 40/30/20/10 split to the listed recipients' pull-payment balances.
    /// @dev    Treasury share absorbs any bps rounding so `sum(shares) == totalFee`.
    ///         Validator slice is split evenly, remainder → `validators[0]` for
    ///         deterministic behavior.
    function distribute(
        uint256 jobId,
        address agent,
        address provider,
        address[] calldata validators,
        uint256 totalFee
    ) external {
        if (totalFee == 0) revert ZeroAmount();
        if (agent == address(0)) revert ZeroAgent();
        if (provider == address(0)) revert ZeroProvider();
        if (validators.length == 0) revert EmptyValidators();

        bool ok = token.transferFrom(msg.sender, address(this), totalFee);
        if (!ok) revert TransferFailed();

        uint256 agentShare = (totalFee * AGENT_BPS) / BPS_DENOM;
        uint256 providerShare = (totalFee * PROVIDER_BPS) / BPS_DENOM;
        uint256 validatorTotal = (totalFee * VALIDATOR_BPS) / BPS_DENOM;
        // Treasury absorbs any leftover so the sum always equals totalFee.
        uint256 treasuryShare = totalFee - agentShare - providerShare - validatorTotal;

        earningsOf[agent] += agentShare;
        earningsOf[provider] += providerShare;
        earningsOf[treasury] += treasuryShare;

        uint256 perValidator = validatorTotal / validators.length;
        uint256 validatorRemainder = validatorTotal - (perValidator * validators.length);
        for (uint256 i = 0; i < validators.length; i++) {
            earningsOf[validators[i]] += perValidator;
        }
        if (validatorRemainder > 0) {
            earningsOf[validators[0]] += validatorRemainder;
        }

        totalDistributed += totalFee;

        emit FeesDistributed(
            jobId,
            agent,
            provider,
            validators,
            totalFee,
            agentShare,
            providerShare,
            validatorTotal,
            treasuryShare
        );
    }

    /// @notice Withdraw all pending earnings for msg.sender.
    function claim() external returns (uint256 amount) {
        amount = earningsOf[msg.sender];
        if (amount == 0) revert NothingToClaim();
        earningsOf[msg.sender] = 0;
        bool ok = token.transfer(msg.sender, amount);
        if (!ok) revert TransferFailed();
        emit FeesClaimed(msg.sender, amount);
    }

    /// @notice Read pending earnings for `account` without claiming.
    function pendingOf(address account) external view returns (uint256) {
        return earningsOf[account];
    }
}
