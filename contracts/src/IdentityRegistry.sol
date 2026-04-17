// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

interface IERC20Minimal {
    function transfer(address to, uint256 amount) external returns (bool);
    function transferFrom(address from, address to, uint256 amount) external returns (bool);
}

/// @title IdentityRegistry
/// @notice ERC-8004 identity registry with a soulbound ERC-721 passport surface.
contract IdentityRegistry {
    uint8 public constant TIER_PROTOCOL = 0;
    uint8 public constant TIER_SOVEREIGN = 1;
    uint8 public constant TIER_WORKER = 2;
    uint8 public constant TIER_EDGE = 3;

    uint64 public constant PROMPT_UPDATE_DELAY = 1 days;
    uint64 public constant WITHDRAW_COOLDOWN = 7 days;
    uint256 public constant WORKER_STAKE_THRESHOLD = 5_000 ether;
    uint256 public constant SOVEREIGN_STAKE_THRESHOLD = 25_000 ether;

    struct PassportData {
        uint64 capabilityList;
        uint8 tier;
        bytes32 systemPromptHash;
        bytes32 teeAttestation;
        uint64 teeExpiry;
        uint64 registeredBlock;
        string agentCardUri;
    }

    struct AgentPassport {
        uint256 passportId;
        address owner;
        uint64 capabilityBitmask;
        uint8 tier;
        bytes32 systemPromptHash;
        bytes32 teeAttestation;
        uint64 teeExpiry;
        uint64 registeredBlock;
        string agentCardUri;
        uint256 totalStake;
    }

    struct PendingPromptUpdate {
        bytes32 newHash;
        uint64 executableAt;
    }

    struct DomainStake {
        string domain;
        uint256 amount;
        uint64 cooldownEndsAt;
    }

    string public name;
    string public symbol;
    address public admin;
    IERC20Minimal public immutable stakeToken;

    mapping(address => bool) public registrars;
    mapping(uint256 => PassportData) public passports;
    mapping(address => uint256) public ownerToPassportId;
    mapping(uint256 => PendingPromptUpdate) public pendingPromptUpdates;
    mapping(uint256 => uint256) public totalStakeOf;

    mapping(uint256 => address) private _owners;
    mapping(address => uint256) private _balances;
    mapping(uint256 => mapping(bytes32 => DomainStake)) private _domainStakes;
    mapping(uint256 => bytes32[]) private _domainStakeKeys;
    uint256 private _nextPassportId = 1;

    event Transfer(address indexed from, address indexed to, uint256 indexed tokenId);
    event Locked(uint256 indexed tokenId);

    event RegistrarSet(address indexed registrar, bool allowed);
    event PassportMinted(uint256 indexed passportId, address indexed owner, uint8 tier, uint64 capabilities);
    event AgentRegistered(address indexed agent, uint256 indexed passportId, uint8 tier, uint64 capabilities);
    event CapabilitiesUpdated(uint256 indexed passportId, uint64 capabilities);
    event PromptHashUpdateScheduled(uint256 indexed passportId, bytes32 newHash, uint64 executableAt);
    event PromptHashUpdated(uint256 indexed passportId, bytes32 newHash);
    event TeeAttestationUpdated(uint256 indexed passportId, bytes32 attestationHash, uint64 expiry);
    event AgentCardUriUpdated(uint256 indexed passportId, string agentCardUri);
    event DomainStakeUpdated(uint256 indexed passportId, string domain, uint256 amount, uint64 cooldownEndsAt);
    event TierUpdated(uint256 indexed passportId, uint8 oldTier, uint8 newTier);
    event PassportRevoked(uint256 indexed passportId, address indexed owner);

    error NotAdmin();
    error NotPassportOwner();
    error NotAuthorizedRegistrar();
    error AlreadyRegistered();
    error InvalidTier();
    error InvalidAmount();
    error NonexistentPassport();
    error Soulbound();
    error NoChange();
    error PromptUpdateNotReady();
    error StakingDisabled();
    error CooldownActive(uint64 availableAt);
    error InsufficientStake();

    constructor(address initialAdmin, address stakeToken_) {
        admin = initialAdmin == address(0) ? msg.sender : initialAdmin;
        name = "Korai Passport";
        symbol = "KPASS";
        stakeToken = IERC20Minimal(stakeToken_);
        registrars[admin] = true;
        emit RegistrarSet(admin, true);
    }

    modifier onlyAdmin() {
        if (msg.sender != admin) revert NotAdmin();
        _;
    }

    modifier onlyPassportOwner(uint256 passportId) {
        if (ownerOf(passportId) != msg.sender) revert NotPassportOwner();
        _;
    }

    function setRegistrar(address registrar, bool allowed) external onlyAdmin {
        registrars[registrar] = allowed;
        emit RegistrarSet(registrar, allowed);
    }

    function registerPassport(
        address owner_,
        uint64 capabilityBitmask,
        bytes32 systemPromptHash,
        bytes32 teeAttestation,
        uint64 teeExpiry
    ) external returns (uint256 passportId) {
        if (!_isRegistrarFor(owner_)) revert NotAuthorizedRegistrar();
        passportId = _register(
            owner_,
            capabilityBitmask,
            _tierFromStake(0, TIER_EDGE),
            systemPromptHash,
            teeAttestation,
            teeExpiry,
            ""
        );
    }

    function register(
        address agent,
        uint64 capabilityList,
        uint8 tier,
        bytes32 systemPromptHash,
        string calldata agentCardUri
    ) external returns (uint256 passportId) {
        if (!_isRegistrarFor(agent)) revert NotAuthorizedRegistrar();
        if (tier > TIER_EDGE) revert InvalidTier();
        passportId = _register(agent, capabilityList, tier, systemPromptHash, bytes32(0), 0, agentCardUri);
    }

    function balanceOf(address owner_) public view returns (uint256) {
        if (owner_ == address(0)) revert NonexistentPassport();
        return _balances[owner_];
    }

    function ownerOf(uint256 passportId) public view returns (address owner_) {
        owner_ = _owners[passportId];
        if (owner_ == address(0)) revert NonexistentPassport();
    }

    function tokenURI(uint256 passportId) external view returns (string memory) {
        ownerOf(passportId);
        return passports[passportId].agentCardUri;
    }

    function passportIdOf(address owner_) external view returns (uint256) {
        return ownerToPassportId[owner_];
    }

    function locked(uint256 tokenId) external view returns (bool) {
        ownerOf(tokenId);
        return true;
    }

    function supportsInterface(bytes4 interfaceId) external pure returns (bool) {
        return interfaceId == 0x01ffc9a7
            || interfaceId == 0x80ac58cd
            || interfaceId == 0x5b5e139f
            || interfaceId == 0xb45a3c0e;
    }

    function approve(address, uint256) external pure {
        revert Soulbound();
    }

    function setApprovalForAll(address, bool) external pure {
        revert Soulbound();
    }

    function getApproved(uint256 tokenId) external view returns (address) {
        ownerOf(tokenId);
        return address(0);
    }

    function isApprovedForAll(address, address) external pure returns (bool) {
        return false;
    }

    function transferFrom(address, address, uint256) external pure {
        revert Soulbound();
    }

    function safeTransferFrom(address, address, uint256) external pure {
        revert Soulbound();
    }

    function safeTransferFrom(address, address, uint256, bytes calldata) external pure {
        revert Soulbound();
    }

    function revokePassport(uint256 passportId) external {
        address owner_ = ownerOf(passportId);
        if (msg.sender != admin && msg.sender != owner_) revert NotPassportOwner();

        _balances[owner_] -= 1;
        delete ownerToPassportId[owner_];
        delete _owners[passportId];

        emit Transfer(owner_, address(0), passportId);
        emit PassportRevoked(passportId, owner_);
    }

    function updateCapabilities(uint256 passportId, uint64 newCapabilities) external onlyPassportOwner(passportId) {
        passports[passportId].capabilityList = newCapabilities;
        emit CapabilitiesUpdated(passportId, newCapabilities);
    }

    function updatePromptHash(uint256 passportId, bytes32 newHash) external onlyPassportOwner(passportId) {
        _scheduleOrFinalizePromptUpdate(passportId, newHash);
    }

    function updateSystemPromptHash(uint256 passportId, bytes32 newHash) external onlyPassportOwner(passportId) {
        _scheduleOrFinalizePromptUpdate(passportId, newHash);
    }

    function updateAttestation(uint256 passportId, bytes32 attestationHash, uint64 expiry)
        external
        onlyPassportOwner(passportId)
    {
        passports[passportId].teeAttestation = attestationHash;
        passports[passportId].teeExpiry = expiry;
        emit TeeAttestationUpdated(passportId, attestationHash, expiry);
    }

    function updateTeeAttestation(uint256 passportId, bytes32 attestationHash)
        external
        onlyPassportOwner(passportId)
    {
        passports[passportId].teeAttestation = attestationHash;
        emit TeeAttestationUpdated(passportId, attestationHash, passports[passportId].teeExpiry);
    }

    function updateAgentCardUri(uint256 passportId, string calldata newUri)
        external
        onlyPassportOwner(passportId)
    {
        passports[passportId].agentCardUri = newUri;
        emit AgentCardUriUpdated(passportId, newUri);
    }

    function hasCapability(uint256 passportId, uint8 capBit) external view returns (bool) {
        ownerOf(passportId);
        return (passports[passportId].capabilityList & (uint64(1) << capBit)) != 0;
    }

    function hasCapability(uint256 passportId, uint64 capabilityMask) external view returns (bool) {
        ownerOf(passportId);
        return (passports[passportId].capabilityList & capabilityMask) != 0;
    }

    function stakeIntoDomain(uint256 passportId, string calldata domain, uint256 amount)
        external
        onlyPassportOwner(passportId)
    {
        if (amount == 0) revert InvalidAmount();
        if (address(stakeToken) == address(0)) revert StakingDisabled();

        bool ok = stakeToken.transferFrom(msg.sender, address(this), amount);
        require(ok, "transferFrom failed");

        bytes32 domainKey = keccak256(bytes(domain));
        DomainStake storage stakeData = _domainStakes[passportId][domainKey];
        if (bytes(stakeData.domain).length == 0) {
            stakeData.domain = domain;
            _domainStakeKeys[passportId].push(domainKey);
        }

        stakeData.amount += amount;
        stakeData.cooldownEndsAt = uint64(block.timestamp + WITHDRAW_COOLDOWN);
        totalStakeOf[passportId] += amount;

        emit DomainStakeUpdated(passportId, domain, stakeData.amount, stakeData.cooldownEndsAt);
        _syncTier(passportId);
    }

    function withdrawFromDomain(uint256 passportId, string calldata domain, uint256 amount)
        external
        onlyPassportOwner(passportId)
    {
        if (amount == 0) revert InvalidAmount();
        if (address(stakeToken) == address(0)) revert StakingDisabled();

        bytes32 domainKey = keccak256(bytes(domain));
        DomainStake storage stakeData = _domainStakes[passportId][domainKey];
        if (stakeData.amount < amount) revert InsufficientStake();
        if (block.timestamp < stakeData.cooldownEndsAt) revert CooldownActive(stakeData.cooldownEndsAt);

        stakeData.amount -= amount;
        totalStakeOf[passportId] -= amount;

        bool ok = stakeToken.transfer(msg.sender, amount);
        require(ok, "transfer failed");

        emit DomainStakeUpdated(passportId, domain, stakeData.amount, stakeData.cooldownEndsAt);
        _syncTier(passportId);
    }

    function getDomainStake(uint256 passportId, string calldata domain)
        external
        view
        returns (uint256 amount, uint64 cooldownEndsAt)
    {
        ownerOf(passportId);
        DomainStake storage stakeData = _domainStakes[passportId][keccak256(bytes(domain))];
        return (stakeData.amount, stakeData.cooldownEndsAt);
    }

    function getDomainStakes(uint256 passportId) external view returns (DomainStake[] memory stakes) {
        ownerOf(passportId);

        bytes32[] storage keys = _domainStakeKeys[passportId];
        stakes = new DomainStake[](keys.length);
        for (uint256 i = 0; i < keys.length; i++) {
            DomainStake storage stakeData = _domainStakes[passportId][keys[i]];
            stakes[i] = DomainStake({
                domain: stakeData.domain,
                amount: stakeData.amount,
                cooldownEndsAt: stakeData.cooldownEndsAt
            });
        }
    }

    function getPassport(uint256 passportId) external view returns (AgentPassport memory passport) {
        address owner_ = ownerOf(passportId);
        PassportData storage data = passports[passportId];
        passport = AgentPassport({
            passportId: passportId,
            owner: owner_,
            capabilityBitmask: data.capabilityList,
            tier: data.tier,
            systemPromptHash: data.systemPromptHash,
            teeAttestation: data.teeAttestation,
            teeExpiry: data.teeExpiry,
            registeredBlock: data.registeredBlock,
            agentCardUri: data.agentCardUri,
            totalStake: totalStakeOf[passportId]
        });
    }

    function getTier(uint256 passportId) external view returns (uint8) {
        ownerOf(passportId);
        return passports[passportId].tier;
    }

    function demoteTier(uint256 passportId) external onlyAdmin {
        _setTier(passportId, TIER_EDGE);
    }

    function demoteTier(uint256 passportId, uint8 newTier) external onlyAdmin {
        if (newTier > TIER_EDGE) revert InvalidTier();
        _setTier(passportId, newTier);
    }

    function _register(
        address owner_,
        uint64 capabilityList,
        uint8 tier,
        bytes32 systemPromptHash,
        bytes32 teeAttestation,
        uint64 teeExpiry,
        string memory agentCardUri
    ) internal returns (uint256 passportId) {
        if (owner_ == address(0)) revert NonexistentPassport();
        if (ownerToPassportId[owner_] != 0) revert AlreadyRegistered();

        passportId = _nextPassportId++;
        _owners[passportId] = owner_;
        _balances[owner_] += 1;
        ownerToPassportId[owner_] = passportId;

        passports[passportId] = PassportData({
            capabilityList: capabilityList,
            tier: tier,
            systemPromptHash: systemPromptHash,
            teeAttestation: teeAttestation,
            teeExpiry: teeExpiry,
            registeredBlock: uint64(block.number),
            agentCardUri: agentCardUri
        });

        emit Transfer(address(0), owner_, passportId);
        emit Locked(passportId);
        emit PassportMinted(passportId, owner_, tier, capabilityList);
        emit AgentRegistered(owner_, passportId, tier, capabilityList);
    }

    function _scheduleOrFinalizePromptUpdate(uint256 passportId, bytes32 newHash) internal {
        PassportData storage passport = passports[passportId];
        if (passport.systemPromptHash == newHash) revert NoChange();

        PendingPromptUpdate storage pending = pendingPromptUpdates[passportId];
        if (pending.newHash != newHash) {
            pending.newHash = newHash;
            pending.executableAt = uint64(block.timestamp + PROMPT_UPDATE_DELAY);
            emit PromptHashUpdateScheduled(passportId, newHash, pending.executableAt);
            return;
        }

        if (block.timestamp < pending.executableAt) revert PromptUpdateNotReady();

        passport.systemPromptHash = newHash;
        delete pendingPromptUpdates[passportId];
        emit PromptHashUpdated(passportId, newHash);
    }

    function _syncTier(uint256 passportId) internal {
        PassportData storage passport = passports[passportId];
        uint8 newTier = _tierFromStake(totalStakeOf[passportId], passport.tier);
        if (newTier != passport.tier) {
            uint8 oldTier = passport.tier;
            passport.tier = newTier;
            emit TierUpdated(passportId, oldTier, newTier);
        }
    }

    function _setTier(uint256 passportId, uint8 newTier) internal {
        ownerOf(passportId);
        PassportData storage passport = passports[passportId];
        uint8 oldTier = passport.tier;
        if (oldTier == newTier) revert NoChange();
        passport.tier = newTier;
        emit TierUpdated(passportId, oldTier, newTier);
    }

    function _tierFromStake(uint256 totalStake, uint8 currentTier) internal pure returns (uint8) {
        if (currentTier == TIER_PROTOCOL) {
            return TIER_PROTOCOL;
        }
        if (totalStake >= SOVEREIGN_STAKE_THRESHOLD) {
            return TIER_SOVEREIGN;
        }
        if (totalStake >= WORKER_STAKE_THRESHOLD) {
            return TIER_WORKER;
        }
        return TIER_EDGE;
    }

    function _isRegistrarFor(address owner_) internal view returns (bool) {
        return msg.sender == owner_ || msg.sender == admin || registrars[msg.sender];
    }
}
