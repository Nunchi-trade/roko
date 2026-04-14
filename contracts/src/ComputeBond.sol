// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import { IERC20 } from "@openzeppelin/contracts/token/ERC20/IERC20.sol";

/// @title ComputeBond — data provider stake + slash registry.
/// @notice Data providers register by bonding ERC20 tokens. Tier derives from bond size.
///         Authorized slashers (e.g. `BountyMarket`, `ConsortiumValidator`) can slash
///         providers with reason codes. Unbond has a 7-day delay to prevent a provider
///         from fleeing with their stake when a slash is about to land.
/// @dev    Modeled after `WorkerRegistry`'s stake + reputation + reason-coded slash
///         pattern, but without EMA reputation (providers are binary: active or not).
contract ComputeBond {
    enum ProviderTier { None, Bronze, Silver, Gold, Platinum }

    struct Provider {
        uint256 bond;
        uint64 registeredAt;
        uint64 unbondRequestedAt;  // 0 = no unbond requested
        uint64 jobsCompleted;
        uint64 jobsSlashed;
        bool exists;
    }

    IERC20 public immutable stakeToken;
    address public owner;
    mapping(address => bool) public authorized;

    uint256 public constant MIN_BOND = 10_000 ether;
    uint256 public constant SILVER_THRESHOLD = 50_000 ether;
    uint256 public constant GOLD_THRESHOLD = 200_000 ether;
    uint256 public constant PLATINUM_THRESHOLD = 1_000_000 ether;
    uint64 public constant UNBOND_DELAY = 7 days;

    /// @dev Slash reason codes → basis points of current bond.
    uint8 public constant SLASH_FRAUD = 1;        // 100% (10_000 bps)
    uint8 public constant SLASH_INCOMPLETE = 2;   //  10% ( 1_000 bps)
    uint8 public constant SLASH_STALE_DATA = 3;   //   5% (   500 bps)
    uint8 public constant SLASH_UNAVAILABLE = 4;  //   2% (   200 bps)

    mapping(address => Provider) private _providers;
    address[] private _registered;

    event ProviderRegistered(address indexed provider, uint256 bond);
    event BondIncreased(address indexed provider, uint256 amount, uint256 newBond);
    event UnbondRequested(address indexed provider, uint64 availableAt);
    event BondWithdrawn(address indexed provider, uint256 amount);
    event ProviderSlashed(address indexed provider, uint8 reasonCode, uint256 amount, uint256 newBond);
    event JobCredited(address indexed provider, uint64 newCompletedCount);
    event AuthorizedSet(address indexed caller, bool allowed);
    event OwnerTransferred(address indexed previous, address indexed newOwner);

    error InsufficientBond();
    error AlreadyRegistered();
    error NotRegistered();
    error NotAuthorizedCaller();
    error UnbondTooEarly();
    error NoUnbondRequested();
    error InvalidReason();
    error OnlyOwner();
    error ZeroAddress();
    error TransferFailed();

    modifier onlyOwner() {
        if (msg.sender != owner) revert OnlyOwner();
        _;
    }

    modifier onlyAuthorized() {
        if (!authorized[msg.sender]) revert NotAuthorizedCaller();
        _;
    }

    constructor(address stakeToken_, address owner_) {
        if (stakeToken_ == address(0) || owner_ == address(0)) revert ZeroAddress();
        stakeToken = IERC20(stakeToken_);
        owner = owner_;
    }

    /// @notice Register as a data provider with an initial bond ≥ `MIN_BOND`.
    function register(uint256 bondAmount) external {
        if (_providers[msg.sender].exists) revert AlreadyRegistered();
        if (bondAmount < MIN_BOND) revert InsufficientBond();

        bool ok = stakeToken.transferFrom(msg.sender, address(this), bondAmount);
        if (!ok) revert TransferFailed();

        _providers[msg.sender] = Provider({
            bond: bondAmount,
            registeredAt: uint64(block.timestamp),
            unbondRequestedAt: 0,
            jobsCompleted: 0,
            jobsSlashed: 0,
            exists: true
        });
        _registered.push(msg.sender);

        emit ProviderRegistered(msg.sender, bondAmount);
    }

    /// @notice Top up bond — anyone can call for themselves (no min-bond re-check
    ///         because adding more is always safe).
    function bond(uint256 amount) external {
        Provider storage p = _providers[msg.sender];
        if (!p.exists) revert NotRegistered();

        bool ok = stakeToken.transferFrom(msg.sender, address(this), amount);
        if (!ok) revert TransferFailed();

        p.bond += amount;
        emit BondIncreased(msg.sender, amount, p.bond);
    }

    /// @notice Mark the provider as unbonding. Locks withdrawal for `UNBOND_DELAY`.
    function requestUnbond() external {
        Provider storage p = _providers[msg.sender];
        if (!p.exists) revert NotRegistered();
        p.unbondRequestedAt = uint64(block.timestamp);
        emit UnbondRequested(msg.sender, p.unbondRequestedAt + UNBOND_DELAY);
    }

    /// @notice Withdraw the full bond after the unbond delay has elapsed.
    function withdraw() external {
        Provider storage p = _providers[msg.sender];
        if (!p.exists) revert NotRegistered();
        if (p.unbondRequestedAt == 0) revert NoUnbondRequested();
        if (block.timestamp < p.unbondRequestedAt + UNBOND_DELAY) revert UnbondTooEarly();

        uint256 amount = p.bond;
        p.bond = 0;
        p.unbondRequestedAt = 0;

        bool ok = stakeToken.transfer(msg.sender, amount);
        if (!ok) revert TransferFailed();
        emit BondWithdrawn(msg.sender, amount);
    }

    /// @notice Authorized caller (BountyMarket / ConsortiumValidator / etc.) records
    ///         a completed job against a provider.
    function creditJob(address provider) external onlyAuthorized {
        Provider storage p = _providers[provider];
        if (!p.exists) revert NotRegistered();
        p.jobsCompleted += 1;
        emit JobCredited(provider, p.jobsCompleted);
    }

    /// @notice Authorized caller slashes a provider's bond per a reason code.
    ///         Slashed tokens remain in this contract; treasury/burn routing is a
    ///         downstream decision.
    function slash(address provider, uint8 reasonCode) external onlyAuthorized {
        Provider storage p = _providers[provider];
        if (!p.exists) revert NotRegistered();

        uint256 bps = _reasonBps(reasonCode);
        uint256 slashAmount = (p.bond * bps) / 10_000;
        if (slashAmount > p.bond) slashAmount = p.bond;

        p.bond -= slashAmount;
        p.jobsSlashed += 1;

        emit ProviderSlashed(provider, reasonCode, slashAmount, p.bond);
    }

    function setAuthorized(address caller, bool allowed) external onlyOwner {
        if (caller == address(0)) revert ZeroAddress();
        authorized[caller] = allowed;
        emit AuthorizedSet(caller, allowed);
    }

    function transferOwnership(address newOwner) external onlyOwner {
        if (newOwner == address(0)) revert ZeroAddress();
        address prev = owner;
        owner = newOwner;
        emit OwnerTransferred(prev, newOwner);
    }

    /* ---------------- views ---------------- */

    function tierOf(address provider) external view returns (ProviderTier) {
        Provider storage p = _providers[provider];
        if (!p.exists) return ProviderTier.None;
        uint256 b = p.bond;
        if (b >= PLATINUM_THRESHOLD) return ProviderTier.Platinum;
        if (b >= GOLD_THRESHOLD) return ProviderTier.Gold;
        if (b >= SILVER_THRESHOLD) return ProviderTier.Silver;
        if (b >= MIN_BOND) return ProviderTier.Bronze;
        return ProviderTier.None;
    }

    function isActive(address provider) external view returns (bool) {
        Provider storage p = _providers[provider];
        return p.exists && p.bond >= MIN_BOND && p.unbondRequestedAt == 0;
    }

    function getProvider(address provider) external view returns (Provider memory) {
        return _providers[provider];
    }

    function registeredCount() external view returns (uint256) {
        return _registered.length;
    }

    function registeredAtIndex(uint256 index) external view returns (address) {
        return _registered[index];
    }

    /* ---------------- internal ---------------- */

    function _reasonBps(uint8 reasonCode) internal pure returns (uint256) {
        if (reasonCode == SLASH_FRAUD) return 10_000;       // 100%
        if (reasonCode == SLASH_INCOMPLETE) return 1_000;   //  10%
        if (reasonCode == SLASH_STALE_DATA) return 500;     //   5%
        if (reasonCode == SLASH_UNAVAILABLE) return 200;    //   2%
        revert InvalidReason();
    }
}
