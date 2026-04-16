// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import { IERC20 } from "@openzeppelin/contracts/token/ERC20/IERC20.sol";

/// @title FeeDistributor
/// @notice Splits a funded payment across validators, data providers, agent, and treasury.
contract FeeDistributor {
    uint256 public constant VALIDATOR_BPS = 4_000;
    uint256 public constant DATA_PROVIDER_BPS = 3_000;
    uint256 public constant AGENT_BPS = 2_000;
    uint256 public constant TREASURY_BPS = 1_000;
    uint256 public constant BPS_DENOMINATOR = 10_000;

    IERC20 public immutable rewardToken;
    address public immutable treasury;

    mapping(address => uint256) public cumulativeEarnings;

    event FeesDistributed(
        uint256 indexed jobId,
        uint256 amount,
        address indexed winner,
        uint256 validatorShare,
        uint256 dataShare,
        uint256 agentShare,
        uint256 treasuryShare
    );
    event EarningsCredited(address indexed participant, uint256 amount);

    error ZeroTreasury();
    error TransferFailed();

    constructor(address rewardToken_, address treasury_) {
        if (treasury_ == address(0)) revert ZeroTreasury();
        rewardToken = IERC20(rewardToken_);
        treasury = treasury_;
    }

    function distribute(
        uint256 jobId,
        uint256 amount,
        address winner,
        address[] calldata validators,
        address[] calldata dataProviders
    ) external {
        if (!rewardToken.transferFrom(msg.sender, address(this), amount)) {
            revert TransferFailed();
        }

        uint256 validatorShare = (amount * VALIDATOR_BPS) / BPS_DENOMINATOR;
        uint256 dataShare = (amount * DATA_PROVIDER_BPS) / BPS_DENOMINATOR;
        uint256 agentShare = (amount * AGENT_BPS) / BPS_DENOMINATOR;
        uint256 treasuryShare = amount - validatorShare - dataShare - agentShare;

        if (validators.length == 0) {
            treasuryShare += validatorShare;
            validatorShare = 0;
        } else {
            _creditGroup(validators, validatorShare);
        }

        if (dataProviders.length == 0) {
            treasuryShare += dataShare;
            dataShare = 0;
        } else {
            _creditGroup(dataProviders, dataShare);
        }

        _credit(winner, agentShare);
        _credit(treasury, treasuryShare);

        emit FeesDistributed(
            jobId,
            amount,
            winner,
            validatorShare,
            dataShare,
            agentShare,
            treasuryShare
        );
    }

    function _creditGroup(address[] calldata recipients, uint256 totalAmount) internal {
        uint256 perRecipient = totalAmount / recipients.length;
        uint256 remainder = totalAmount % recipients.length;
        for (uint256 i = 0; i < recipients.length; i++) {
            uint256 amount = perRecipient;
            if (i < remainder) {
                amount += 1;
            }
            _credit(recipients[i], amount);
        }
    }

    function _credit(address recipient, uint256 amount) internal {
        cumulativeEarnings[recipient] += amount;
        emit EarningsCredited(recipient, amount);
        if (!rewardToken.transfer(recipient, amount)) {
            revert TransferFailed();
        }
    }
}
