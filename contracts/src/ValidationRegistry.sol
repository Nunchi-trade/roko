// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import { IdentityRegistry } from "./IdentityRegistry.sol";

/// @title ValidationRegistry
/// @notice ERC-8004 validation registry for work proofs and validator attestations.
contract ValidationRegistry {
    enum ValidatorType {
        ReputationBased,
        StakeSecuredReExecution,
        ZkMLProof,
        TeeOracle
    }

    struct ValidationRequest {
        uint256 requesterPassportId;
        bytes32 workHash;
        bytes32 taskId;
        ValidatorType validatorType;
        uint64 requestedBlock;
        bool resolved;
    }

    struct ValidationAttestation {
        uint256 validatorPassportId;
        bytes32 workHash;
        bool approved;
        bytes32 evidenceHash;
        uint64 attestedBlock;
    }

    struct WorkProof {
        uint256 passportId;
        bytes32 jobHash;
        bytes32 deliverableMerkleRoot;
        uint8[] gateResults;
        bytes clearingCert;
        uint64 blockNumber;
        uint64 timestamp;
    }

    struct GateStats {
        uint64 passed;
        uint64 total;
    }

    IdentityRegistry public immutable identityRegistry;
    address public owner;

    mapping(bytes32 => ValidationRequest) public requests;
    mapping(address => bool) public authorizedSubmitters;

    mapping(bytes32 => ValidationAttestation[]) private _attestations;
    mapping(bytes32 => WorkProof) private _workProofs;
    mapping(bytes32 => bool) private _hasWorkProof;
    mapping(uint256 => bytes32[]) private _passportJobHashes;
    mapping(uint256 => GateStats) private _gateStats;
    uint256 private _requestNonce = 1;

    event AuthorizedSubmitterSet(address indexed submitter, bool allowed);
    event WorkProofSubmitted(uint256 indexed passportId, bytes32 indexed jobHash, bytes32 deliverableMerkleRoot);
    event ValidationRequested(
        bytes32 indexed requestId,
        uint256 indexed requester,
        bytes32 workHash,
        ValidatorType validatorType
    );
    event AttestationSubmitted(bytes32 indexed requestId, uint256 indexed validator, bool approved);

    error NotOwner();
    error NotAuthorized();
    error UnknownPassport();
    error WorkProofAlreadyExists();
    error UnknownWorkProof();
    error UnknownRequest();
    error RequestResolved();

    constructor(address identityRegistry_, address initialOwner) {
        identityRegistry = IdentityRegistry(identityRegistry_);
        owner = initialOwner == address(0) ? msg.sender : initialOwner;
    }

    modifier onlyOwner() {
        if (msg.sender != owner) revert NotOwner();
        _;
    }

    function setAuthorizedSubmitter(address submitter, bool allowed) external onlyOwner {
        authorizedSubmitters[submitter] = allowed;
        emit AuthorizedSubmitterSet(submitter, allowed);
    }

    function submitWorkProof(
        uint256 passportId,
        bytes32 jobHash,
        bytes32 deliverableMerkleRoot,
        uint8[] calldata gateResults,
        bytes calldata clearingCert
    ) external {
        if (!_canSubmitProof(passportId, msg.sender)) revert NotAuthorized();
        if (_hasWorkProof[jobHash]) revert WorkProofAlreadyExists();

        uint8[] memory gateResultsCopy = gateResults;
        bytes memory clearingCertCopy = clearingCert;

        _workProofs[jobHash] = WorkProof({
            passportId: passportId,
            jobHash: jobHash,
            deliverableMerkleRoot: deliverableMerkleRoot,
            gateResults: gateResultsCopy,
            clearingCert: clearingCertCopy,
            blockNumber: uint64(block.number),
            timestamp: uint64(block.timestamp)
        });
        _hasWorkProof[jobHash] = true;
        _passportJobHashes[passportId].push(jobHash);

        GateStats storage stats = _gateStats[passportId];
        for (uint256 i = 0; i < gateResults.length; i++) {
            stats.total += 1;
            if (gateResults[i] != 0) {
                stats.passed += 1;
            }
        }

        emit WorkProofSubmitted(passportId, jobHash, deliverableMerkleRoot);
    }

    function verifyWork(bytes32 jobHash) external view returns (WorkProof memory) {
        if (!_hasWorkProof[jobHash]) revert UnknownWorkProof();
        return _workProofs[jobHash];
    }

    function getWorkProofs(uint256 passportId, uint64 fromBlock, uint64 toBlock)
        external
        view
        returns (WorkProof[] memory proofs)
    {
        _requirePassport(passportId);

        bytes32[] storage jobHashes = _passportJobHashes[passportId];
        uint256 count;
        for (uint256 i = 0; i < jobHashes.length; i++) {
            WorkProof storage proof = _workProofs[jobHashes[i]];
            if (proof.blockNumber >= fromBlock && proof.blockNumber <= toBlock) {
                count += 1;
            }
        }

        proofs = new WorkProof[](count);
        uint256 cursor;
        for (uint256 i = 0; i < jobHashes.length; i++) {
            WorkProof storage proof = _workProofs[jobHashes[i]];
            if (proof.blockNumber >= fromBlock && proof.blockNumber <= toBlock) {
                proofs[cursor++] = proof;
            }
        }
    }

    function getGatePassRate(uint256 passportId, string calldata)
        external
        view
        returns (uint256 passRate, uint64 totalJobs)
    {
        _requirePassport(passportId);

        GateStats storage stats = _gateStats[passportId];
        totalJobs = uint64(_passportJobHashes[passportId].length);
        if (stats.total == 0) {
            return (0, totalJobs);
        }
        passRate = (uint256(stats.passed) * 1e18) / uint256(stats.total);
    }

    function requestValidation(bytes32 workHash, bytes32 taskId, ValidatorType validatorType)
        external
        returns (bytes32 requestId)
    {
        uint256 requesterPassportId = identityRegistry.ownerToPassportId(msg.sender);
        if (requesterPassportId == 0) revert UnknownPassport();

        requestId = keccak256(abi.encode(workHash, requesterPassportId, block.number, _requestNonce++));
        requests[requestId] = ValidationRequest({
            requesterPassportId: requesterPassportId,
            workHash: workHash,
            taskId: taskId,
            validatorType: validatorType,
            requestedBlock: uint64(block.number),
            resolved: false
        });

        emit ValidationRequested(requestId, requesterPassportId, workHash, validatorType);
    }

    function submitAttestation(bytes32 requestId, bool approved, bytes32 evidenceHash) external {
        ValidationRequest storage req = requests[requestId];
        if (req.requestedBlock == 0) revert UnknownRequest();
        if (req.resolved) revert RequestResolved();

        uint256 validatorPassportId = identityRegistry.ownerToPassportId(msg.sender);
        if (validatorPassportId == 0) revert UnknownPassport();

        _attestations[requestId].push(
            ValidationAttestation({
                validatorPassportId: validatorPassportId,
                workHash: req.workHash,
                approved: approved,
                evidenceHash: evidenceHash,
                attestedBlock: uint64(block.number)
            })
        );

        if (_attestations[requestId].length >= 3) {
            req.resolved = true;
        }

        emit AttestationSubmitted(requestId, validatorPassportId, approved);
    }

    function getAttestations(bytes32 requestId) external view returns (ValidationAttestation[] memory) {
        return _attestations[requestId];
    }

    function getRequest(bytes32 requestId) external view returns (ValidationRequest memory) {
        ValidationRequest memory request = requests[requestId];
        if (request.requestedBlock == 0) revert UnknownRequest();
        return request;
    }

    function _canSubmitProof(uint256 passportId, address sender) internal view returns (bool) {
        if (authorizedSubmitters[sender]) {
            _requirePassport(passportId);
            return true;
        }

        try identityRegistry.ownerOf(passportId) returns (address owner_) {
            return owner_ == sender;
        } catch {
            revert UnknownPassport();
        }
    }

    function _requirePassport(uint256 passportId) internal view {
        try identityRegistry.ownerOf(passportId) returns (address owner_) {
            if (owner_ == address(0)) revert UnknownPassport();
        } catch {
            revert UnknownPassport();
        }
    }
}
