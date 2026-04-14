// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

/// @title EmergencyPause — standalone pause signaling contract.
/// @notice This contract does **not** modify any other contract. Downstream services
///         (BountyMarket integrators, roko-demo scenario spines, off-chain indexers)
///         observe `isPaused(category)` and halt their state machines voluntarily.
///         Owner can pause globally OR per-category via a `bytes32` key for fine-grained
///         control during a demo or incident.
/// @dev    Example usage: `EmergencyPause.pauseCategory(keccak256("bounty-market"), "demo failure")`
///         freezes the bounty-market flow without halting everything. Consumers decide
///         their own key namespace.
contract EmergencyPause {
    address public owner;

    bool public globallyPaused;
    mapping(bytes32 => bool) public categoryPaused;

    event GlobalPaused(address indexed actor, string reason);
    event GlobalUnpaused(address indexed actor);
    event CategoryPaused(bytes32 indexed category, address indexed actor, string reason);
    event CategoryUnpaused(bytes32 indexed category, address indexed actor);
    event OwnerTransferred(address indexed previous, address indexed newOwner);

    error OnlyOwner();
    error ZeroAddress();

    modifier onlyOwner() {
        if (msg.sender != owner) revert OnlyOwner();
        _;
    }

    constructor(address owner_) {
        if (owner_ == address(0)) revert ZeroAddress();
        owner = owner_;
    }

    function pauseGlobal(string calldata reason) external onlyOwner {
        globallyPaused = true;
        emit GlobalPaused(msg.sender, reason);
    }

    function unpauseGlobal() external onlyOwner {
        globallyPaused = false;
        emit GlobalUnpaused(msg.sender);
    }

    function pauseCategory(bytes32 category, string calldata reason) external onlyOwner {
        categoryPaused[category] = true;
        emit CategoryPaused(category, msg.sender, reason);
    }

    function unpauseCategory(bytes32 category) external onlyOwner {
        categoryPaused[category] = false;
        emit CategoryUnpaused(category, msg.sender);
    }

    function transferOwnership(address newOwner) external onlyOwner {
        if (newOwner == address(0)) revert ZeroAddress();
        address prev = owner;
        owner = newOwner;
        emit OwnerTransferred(prev, newOwner);
    }

    /* ---------------- views ---------------- */

    /// @notice Composite check: true if globally paused OR the specific category is paused.
    function isPaused(bytes32 category) external view returns (bool) {
        return globallyPaused || categoryPaused[category];
    }

    function isGloballyPaused() external view returns (bool) {
        return globallyPaused;
    }

    function isCategoryPaused(bytes32 category) external view returns (bool) {
        return categoryPaused[category];
    }
}
