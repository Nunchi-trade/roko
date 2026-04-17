// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import { IdentityRegistry } from "./IdentityRegistry.sol";

/// @title ReputationRegistry
/// @notice ERC-8004 reputation registry with both authorized-source and peer-feedback flows.
contract ReputationRegistry {
    uint256 public constant SCALE = 1e18;
    uint256 public constant MAX_ALPHA = 3e17;
    uint64 public constant DECAY_PERIOD = 30 days;

    enum ReputationDomain {
        OracleResolution,
        RiskDetection,
        AnomalyFlagging,
        DataIntegrity,
        CrossAppValidation,
        SealedExecution,
        KnowledgeVerification
    }

    struct FeedbackAuthorization {
        uint256 raterPassportId;
        uint256 rateePassportId;
        uint8 domain;
        bool active;
    }

    struct DomainReputation {
        string domain;
        uint256 score;
        uint64 jobCount;
        uint64 lastUpdate;
    }

    struct SlashRecord {
        uint64 blockNumber;
        uint8 violationType;
        uint256 amount;
        string reason;
    }

    IdentityRegistry public immutable identityRegistry;
    address public owner;

    mapping(bytes32 => FeedbackAuthorization) public authorizations;

    mapping(address => bool) private _authorizedFeedbackSources;
    mapping(uint256 => mapping(bytes32 => DomainReputation)) private _reputations;
    mapping(uint256 => bytes32[]) private _domainKeys;
    mapping(uint256 => SlashRecord[]) private _slashHistory;

    event FeedbackSourceSet(address indexed source, bool allowed);
    event FeedbackAuthorized(uint256 indexed rater, uint256 indexed ratee, uint8 domain);
    event FeedbackSubmitted(
        uint256 indexed rater,
        uint256 indexed ratee,
        uint8 domain,
        uint16 score,
        bytes32 jobId,
        uint256 timestamp
    );
    event FeedbackRecorded(
        address indexed source,
        uint256 indexed passportId,
        string domain,
        int256 score,
        bytes32 jobHash,
        string reason,
        uint256 updatedScore
    );
    event DecayApplied(uint256 indexed passportId, string domain, uint256 score);
    event Slashed(uint256 indexed passportId, uint8 violationType, uint256 amount, string reason);

    error NotOwner();
    error NotAuthorized();
    error InvalidScore();
    error InvalidDomain();
    error UnknownPassport();
    error UnauthorizedRater();

    constructor(address identityRegistry_, address initialOwner) {
        identityRegistry = IdentityRegistry(identityRegistry_);
        owner = initialOwner == address(0) ? msg.sender : initialOwner;
    }

    modifier onlyOwner() {
        if (msg.sender != owner) revert NotOwner();
        _;
    }

    modifier onlyFeedbackSource() {
        if (!_authorizedFeedbackSources[msg.sender] && msg.sender != owner) revert NotAuthorized();
        _;
    }

    function isAuthorizedFeedbackSource(address source) external view returns (bool) {
        return _authorizedFeedbackSources[source];
    }

    function addFeedbackSource(address source) external onlyOwner {
        _authorizedFeedbackSources[source] = true;
        emit FeedbackSourceSet(source, true);
    }

    function setFeedbackSource(address source, bool allowed) external onlyOwner {
        _authorizedFeedbackSources[source] = allowed;
        emit FeedbackSourceSet(source, allowed);
    }

    function authorizeFeedback(uint256 raterPassportId, uint256 rateePassportId, uint8 domain)
        external
        onlyFeedbackSource
    {
        if (domain > uint8(ReputationDomain.KnowledgeVerification)) revert InvalidDomain();
        _requirePassport(raterPassportId);
        _requirePassport(rateePassportId);

        bytes32 key = keccak256(abi.encode(raterPassportId, rateePassportId, domain));
        authorizations[key] = FeedbackAuthorization({
            raterPassportId: raterPassportId,
            rateePassportId: rateePassportId,
            domain: domain,
            active: true
        });

        emit FeedbackAuthorized(raterPassportId, rateePassportId, domain);
    }

    function submitFeedback(
        uint256 passportId,
        string calldata domain,
        int256 score,
        bytes32 jobHash,
        string calldata reason
    ) external onlyFeedbackSource {
        if (score < -int256(SCALE) || score > int256(SCALE)) revert InvalidScore();
        _requirePassport(passportId);

        uint256 normalizedScore = uint256(score + int256(SCALE)) / 2;
        uint256 updatedScore = _recordFeedback(passportId, domain, normalizedScore);

        emit FeedbackRecorded(msg.sender, passportId, domain, score, jobHash, reason, updatedScore);
    }

    function submitFeedback(uint256 rateePassportId, uint8 domain, uint16 score, bytes32 jobId) external {
        if (domain > uint8(ReputationDomain.KnowledgeVerification)) revert InvalidDomain();
        if (score > 1000) revert InvalidScore();

        uint256 raterPassportId = identityRegistry.ownerToPassportId(msg.sender);
        if (raterPassportId == 0) revert UnknownPassport();
        _requirePassport(rateePassportId);

        bytes32 key = keccak256(abi.encode(raterPassportId, rateePassportId, domain));
        if (!authorizations[key].active) revert UnauthorizedRater();

        string memory domainName = _domainName(domain);
        _recordFeedback(rateePassportId, domainName, uint256(score) * 1e15);

        emit FeedbackSubmitted(raterPassportId, rateePassportId, domain, score, jobId, block.timestamp);
    }

    function applyDecayTick(uint256 passportId) external {
        _requirePassport(passportId);

        bytes32[] storage keys = _domainKeys[passportId];
        for (uint256 i = 0; i < keys.length; i++) {
            DomainReputation storage rep = _reputations[passportId][keys[i]];
            uint256 decayed = _decay(rep.score, rep.lastUpdate);
            if (decayed != rep.score) {
                rep.score = decayed;
                rep.lastUpdate = uint64(block.timestamp);
                emit DecayApplied(passportId, rep.domain, decayed);
            }
        }
    }

    function slash(uint256 passportId, uint8 violationType, uint256 amount, string calldata reason)
        external
        onlyFeedbackSource
    {
        _requirePassport(passportId);
        _slashHistory[passportId].push(
            SlashRecord({
                blockNumber: uint64(block.number),
                violationType: violationType,
                amount: amount,
                reason: reason
            })
        );

        emit Slashed(passportId, violationType, amount, reason);
    }

    function getReputation(uint256 passportId, string calldata domain)
        external
        view
        returns (uint256 score, uint64 jobCount, uint64 lastUpdate)
    {
        _requirePassport(passportId);

        DomainReputation storage rep = _reputations[passportId][keccak256(bytes(domain))];
        if (bytes(rep.domain).length == 0) {
            return (SCALE / 2, 0, 0);
        }

        return (_decay(rep.score, rep.lastUpdate), rep.jobCount, rep.lastUpdate);
    }

    function getAllReputations(uint256 passportId) external view returns (DomainReputation[] memory reputations) {
        _requirePassport(passportId);

        bytes32[] storage keys = _domainKeys[passportId];
        reputations = new DomainReputation[](keys.length);
        for (uint256 i = 0; i < keys.length; i++) {
            DomainReputation storage rep = _reputations[passportId][keys[i]];
            reputations[i] = DomainReputation({
                domain: rep.domain,
                score: _decay(rep.score, rep.lastUpdate),
                jobCount: rep.jobCount,
                lastUpdate: rep.lastUpdate
            });
        }
    }

    function getSlashHistory(uint256 passportId) external view returns (SlashRecord[] memory history) {
        _requirePassport(passportId);
        return _slashHistory[passportId];
    }

    function _recordFeedback(uint256 passportId, string memory domain, uint256 normalizedScore)
        internal
        returns (uint256 updatedScore)
    {
        bytes32 domainKey = keccak256(bytes(domain));
        DomainReputation storage rep = _reputations[passportId][domainKey];

        if (bytes(rep.domain).length == 0) {
            rep.domain = domain;
            rep.score = SCALE / 2;
            _domainKeys[passportId].push(domainKey);
        }

        uint256 currentScore = _decay(rep.score, rep.lastUpdate);
        uint256 alpha = _adaptiveAlpha(rep.jobCount);
        updatedScore = (alpha * normalizedScore + (SCALE - alpha) * currentScore) / SCALE;

        rep.score = updatedScore;
        rep.jobCount += 1;
        rep.lastUpdate = uint64(block.timestamp);
    }

    function _adaptiveAlpha(uint64 jobCount) internal pure returns (uint256) {
        uint256 adaptive = (2 * SCALE) / (uint256(jobCount) + 1);
        return adaptive < MAX_ALPHA ? adaptive : MAX_ALPHA;
    }

    function _decay(uint256 score, uint64 lastUpdate) internal view returns (uint256) {
        if (lastUpdate == 0 || block.timestamp <= lastUpdate) return score;

        uint256 halvings = (block.timestamp - lastUpdate) / DECAY_PERIOD;
        if (halvings == 0) return score;
        if (halvings > 64) halvings = 64;

        uint256 mid = SCALE / 2;
        uint256 current = score;
        for (uint256 i = 0; i < halvings; i++) {
            if (current > mid) {
                current = mid + (current - mid) / 2;
            } else {
                current = mid - (mid - current) / 2;
            }
        }
        return current;
    }

    function _domainName(uint8 domain) internal pure returns (string memory) {
        if (domain == uint8(ReputationDomain.OracleResolution)) return "OracleResolution";
        if (domain == uint8(ReputationDomain.RiskDetection)) return "RiskDetection";
        if (domain == uint8(ReputationDomain.AnomalyFlagging)) return "AnomalyFlagging";
        if (domain == uint8(ReputationDomain.DataIntegrity)) return "DataIntegrity";
        if (domain == uint8(ReputationDomain.CrossAppValidation)) return "CrossAppValidation";
        if (domain == uint8(ReputationDomain.SealedExecution)) return "SealedExecution";
        return "KnowledgeVerification";
    }

    function _requirePassport(uint256 passportId) internal view {
        try identityRegistry.ownerOf(passportId) returns (address owner_) {
            if (owner_ == address(0)) revert UnknownPassport();
        } catch {
            revert UnknownPassport();
        }
    }
}
