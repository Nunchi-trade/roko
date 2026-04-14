// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

/// @title JobTypeRegistry — bytes32 → JobTemplate mapping for BountyMarket consumers.
/// @notice Owner-gated registry of job type templates. Used by user-posted jobs AND
///         protocol-posted keeper jobs to share a common type system. Deactivation
///         is a tombstone (keeps enumeration stable — no array shifting).
/// @dev    Example types: `keccak256("oracle-update")`, `keccak256("funding-window")`,
///         `keccak256("dpnl-settlement")`. Templates carry minTier, minBounty, and a
///         maximum deadline offset (seconds from postJob time) so consumers can validate
///         job parameters at post time without hardcoding per-type rules.
contract JobTypeRegistry {
    struct JobTemplate {
        string description;
        uint8 minTier;             // WorkerRegistry.Tier as u8
        uint256 minBounty;
        uint64 maxDeadlineOffset;  // seconds allowed between postJob.time and deadline
        bool active;
        string metadataURI;
    }

    address public owner;
    mapping(bytes32 => JobTemplate) private _templates;
    mapping(bytes32 => bool) private _exists;
    bytes32[] private _registeredTypes;

    event JobTypeRegistered(
        bytes32 indexed jobType,
        string description,
        uint8 minTier,
        uint256 minBounty
    );
    event JobTypeUpdated(bytes32 indexed jobType);
    event JobTypeDeactivated(bytes32 indexed jobType);
    event JobTypeReactivated(bytes32 indexed jobType);
    event OwnerTransferred(address indexed previous, address indexed newOwner);

    error OnlyOwner();
    error UnknownJobType();
    error AlreadyRegistered();
    error EmptyDescription();
    error ZeroAddress();

    modifier onlyOwner() {
        if (msg.sender != owner) revert OnlyOwner();
        _;
    }

    constructor(address owner_) {
        if (owner_ == address(0)) revert ZeroAddress();
        owner = owner_;
    }

    function register(
        bytes32 jobType,
        string calldata description,
        uint8 minTier,
        uint256 minBounty,
        uint64 maxDeadlineOffset,
        string calldata metadataURI
    ) external onlyOwner {
        if (_exists[jobType]) revert AlreadyRegistered();
        if (bytes(description).length == 0) revert EmptyDescription();

        _templates[jobType] = JobTemplate({
            description: description,
            minTier: minTier,
            minBounty: minBounty,
            maxDeadlineOffset: maxDeadlineOffset,
            active: true,
            metadataURI: metadataURI
        });
        _exists[jobType] = true;
        _registeredTypes.push(jobType);

        emit JobTypeRegistered(jobType, description, minTier, minBounty);
    }

    function update(
        bytes32 jobType,
        string calldata description,
        uint8 minTier,
        uint256 minBounty,
        uint64 maxDeadlineOffset,
        string calldata metadataURI
    ) external onlyOwner {
        if (!_exists[jobType]) revert UnknownJobType();
        if (bytes(description).length == 0) revert EmptyDescription();

        JobTemplate storage t = _templates[jobType];
        t.description = description;
        t.minTier = minTier;
        t.minBounty = minBounty;
        t.maxDeadlineOffset = maxDeadlineOffset;
        t.metadataURI = metadataURI;
        // Note: `active` is not touched here — use deactivate/reactivate.

        emit JobTypeUpdated(jobType);
    }

    function deactivate(bytes32 jobType) external onlyOwner {
        if (!_exists[jobType]) revert UnknownJobType();
        _templates[jobType].active = false;
        emit JobTypeDeactivated(jobType);
    }

    function reactivate(bytes32 jobType) external onlyOwner {
        if (!_exists[jobType]) revert UnknownJobType();
        _templates[jobType].active = true;
        emit JobTypeReactivated(jobType);
    }

    function transferOwnership(address newOwner) external onlyOwner {
        if (newOwner == address(0)) revert ZeroAddress();
        address prev = owner;
        owner = newOwner;
        emit OwnerTransferred(prev, newOwner);
    }

    /* ---------------- views ---------------- */

    function getTemplate(bytes32 jobType) external view returns (JobTemplate memory) {
        if (!_exists[jobType]) revert UnknownJobType();
        return _templates[jobType];
    }

    function isActive(bytes32 jobType) external view returns (bool) {
        if (!_exists[jobType]) return false;
        return _templates[jobType].active;
    }

    function exists(bytes32 jobType) external view returns (bool) {
        return _exists[jobType];
    }

    function allJobTypes() external view returns (bytes32[] memory) {
        return _registeredTypes;
    }

    function jobTypeCount() external view returns (uint256) {
        return _registeredTypes.length;
    }
}
