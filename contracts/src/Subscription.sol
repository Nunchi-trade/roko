// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import { IERC20 } from "@openzeppelin/contracts/token/ERC20/IERC20.sol";

/// @title Subscription — tier-based ERC20 subscription for data streams.
/// @notice Subscribers pay ERC20 per 30-day period. Owner sets per-tier pricing.
///         Pull-payment model via `transferFrom`. Time-based expiry.
///         Free tier has zero cost. No refund on cancel — paid time still runs out.
/// @dev    CEI pattern throughout; no ReentrancyGuard because only interaction is
///         with the immutable payment token. Upgrade bills the pro-rated differential
///         between old and new tier for the remaining time window.
contract Subscription {
    enum Tier { None, Free, Pro, Enterprise }

    struct SubscriberState {
        Tier tier;
        uint64 expiresAt;
        uint64 createdAt;
        bool cancelled;
    }

    IERC20 public immutable paymentToken;
    address public owner;
    uint64 public constant PERIOD_SECONDS = 30 days;

    mapping(Tier => uint256) public tierPricePerPeriod;
    mapping(address => SubscriberState) private _subscribers;

    event Subscribed(
        address indexed subscriber,
        Tier tier,
        uint256 periods,
        uint64 expiresAt,
        uint256 amountPaid
    );
    event Extended(address indexed subscriber, uint256 periods, uint64 newExpiresAt, uint256 amountPaid);
    event Upgraded(address indexed subscriber, Tier fromTier, Tier toTier, uint256 differentialPaid);
    event Cancelled(address indexed subscriber);
    event TierPriceSet(Tier indexed tier, uint256 pricePerPeriod);
    event OwnerTransferred(address indexed previous, address indexed newOwner);

    error ZeroAddress();
    error InvalidTier();
    error ZeroPeriods();
    error OnlyOwner();
    error TransferFailed();
    error NotSubscribed();
    error NotAnUpgrade();
    error AlreadyExpired();

    modifier onlyOwner() {
        if (msg.sender != owner) revert OnlyOwner();
        _;
    }

    constructor(address paymentToken_, address owner_) {
        if (paymentToken_ == address(0) || owner_ == address(0)) revert ZeroAddress();
        paymentToken = IERC20(paymentToken_);
        owner = owner_;
    }

    /// @notice Subscribe (or re-subscribe) to `tier` for `periods` months.
    ///         Pulls `tierPrice * periods` via transferFrom from msg.sender.
    function subscribe(Tier tier, uint256 periods) external {
        if (tier == Tier.None) revert InvalidTier();
        if (periods == 0) revert ZeroPeriods();

        uint256 amount = tierPricePerPeriod[tier] * periods;
        if (amount > 0) {
            bool ok = paymentToken.transferFrom(msg.sender, address(this), amount);
            if (!ok) revert TransferFailed();
        }

        uint64 expiresAt = uint64(block.timestamp) + uint64(PERIOD_SECONDS * periods);
        _subscribers[msg.sender] = SubscriberState({
            tier: tier,
            expiresAt: expiresAt,
            createdAt: uint64(block.timestamp),
            cancelled: false
        });

        emit Subscribed(msg.sender, tier, periods, expiresAt, amount);
    }

    /// @notice Extend an active subscription by `periods` months at the current tier.
    ///         Re-enables auto-renew if previously cancelled.
    function extend(uint256 periods) external {
        if (periods == 0) revert ZeroPeriods();
        SubscriberState storage s = _subscribers[msg.sender];
        if (s.tier == Tier.None) revert NotSubscribed();
        if (block.timestamp >= s.expiresAt) revert AlreadyExpired();

        uint256 amount = tierPricePerPeriod[s.tier] * periods;
        if (amount > 0) {
            bool ok = paymentToken.transferFrom(msg.sender, address(this), amount);
            if (!ok) revert TransferFailed();
        }

        s.expiresAt += uint64(PERIOD_SECONDS * periods);
        s.cancelled = false;
        emit Extended(msg.sender, periods, s.expiresAt, amount);
    }

    /// @notice Upgrade to a strictly higher tier. Pays pro-rated differential for
    ///         the remaining time: `(newPrice - currentPrice) * remaining / PERIOD_SECONDS`.
    function upgrade(Tier newTier) external {
        SubscriberState storage s = _subscribers[msg.sender];
        if (s.tier == Tier.None) revert NotSubscribed();
        if (newTier == Tier.None) revert InvalidTier();
        if (block.timestamp >= s.expiresAt) revert AlreadyExpired();
        if (uint8(newTier) <= uint8(s.tier)) revert NotAnUpgrade();

        uint256 remaining = s.expiresAt - uint64(block.timestamp);
        uint256 currentPrice = tierPricePerPeriod[s.tier];
        uint256 newPrice = tierPricePerPeriod[newTier];
        uint256 differential = ((newPrice - currentPrice) * remaining) / PERIOD_SECONDS;

        if (differential > 0) {
            bool ok = paymentToken.transferFrom(msg.sender, address(this), differential);
            if (!ok) revert TransferFailed();
        }

        Tier from = s.tier;
        s.tier = newTier;
        emit Upgraded(msg.sender, from, newTier, differential);
    }

    /// @notice Stop auto-renew. The current paid period still runs out. No refund.
    function cancel() external {
        SubscriberState storage s = _subscribers[msg.sender];
        if (s.tier == Tier.None) revert NotSubscribed();
        s.cancelled = true;
        emit Cancelled(msg.sender);
    }

    /// @notice Owner sets the per-period price for a tier (in payment-token units).
    function setTierPrice(Tier tier, uint256 pricePerPeriod) external onlyOwner {
        if (tier == Tier.None) revert InvalidTier();
        tierPricePerPeriod[tier] = pricePerPeriod;
        emit TierPriceSet(tier, pricePerPeriod);
    }

    function transferOwnership(address newOwner) external onlyOwner {
        if (newOwner == address(0)) revert ZeroAddress();
        address prev = owner;
        owner = newOwner;
        emit OwnerTransferred(prev, newOwner);
    }

    /* ---------------- views ---------------- */

    function isActive(address subscriber) external view returns (bool) {
        SubscriberState storage s = _subscribers[subscriber];
        return s.tier != Tier.None && block.timestamp < s.expiresAt;
    }

    function getState(address subscriber) external view returns (SubscriberState memory) {
        return _subscribers[subscriber];
    }

    function getTier(address subscriber) external view returns (Tier) {
        SubscriberState storage s = _subscribers[subscriber];
        if (s.tier == Tier.None || block.timestamp >= s.expiresAt) return Tier.None;
        return s.tier;
    }
}
