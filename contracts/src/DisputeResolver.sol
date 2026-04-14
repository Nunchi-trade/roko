// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import { BountyMarket } from "./BountyMarket.sol";

/// @title DisputeResolver — mediator-as-proxy-resolver for BountyMarket rejections.
/// @notice Becomes the BountyMarket resolver via `setResolver`. Exposes a happy-path
///         passthrough (no dispute) AND a dispute path with a worker evidence window
///         that the mediator can open for contested rejections. If the window expires:
///             - no response → default reject (upholds mediator)
///             - worker responded → default accept (rewards the responder)
///         Permissionless `finalizeExpired` keeps the contract unblockable if the
///         mediator goes offline.
/// @dev    Construction-time only: `mediator` is immutable. No upgrade path in v1.
///         Consumers must `BountyMarket.setResolver(address(thisContract))` before
///         posting jobs they want routed through the dispute flow.
contract DisputeResolver {
    enum DisputeState { None, Pending, Responded, Adjudicated }

    struct Dispute {
        uint64 openedAt;
        bytes32 evidencePointer;
        DisputeState state;
        bool finalOutcome;
    }

    BountyMarket public immutable market;
    address public immutable mediator;
    uint64 public constant EVIDENCE_WINDOW = 48 hours;

    mapping(uint256 => Dispute) private _disputes;

    event DisputeOpened(uint256 indexed jobId, address indexed opener);
    event EvidenceSubmitted(uint256 indexed jobId, address indexed worker, bytes32 pointer);
    event DisputeFinalized(uint256 indexed jobId, bool outcome, address indexed finalizer);
    event PassthroughResolved(uint256 indexed jobId, bool accepted);

    error NotMediator();
    error NotWorker();
    error AlreadyDisputed();
    error NoDispute();
    error WindowNotExpired();
    error WrongState();
    error ZeroAddress();

    constructor(address market_, address mediator_) {
        if (market_ == address(0) || mediator_ == address(0)) revert ZeroAddress();
        market = BountyMarket(market_);
        mediator = mediator_;
    }

    modifier onlyMediator() {
        if (msg.sender != mediator) revert NotMediator();
        _;
    }

    /// @notice Mediator-only happy path. Forwards directly to `BountyMarket.resolve`
    ///         bypassing the dispute state machine. Used for uncontested jobs.
    function passthrough(uint256 jobId, bool accepted) external onlyMediator {
        market.resolve(jobId, accepted);
        emit PassthroughResolved(jobId, accepted);
    }

    /// @notice Mediator-only. Opens a dispute window on a rejection. The worker has
    ///         `EVIDENCE_WINDOW` seconds to submit evidence before finalization.
    function openDispute(uint256 jobId) external onlyMediator {
        Dispute storage d = _disputes[jobId];
        if (d.state != DisputeState.None) revert AlreadyDisputed();
        d.state = DisputeState.Pending;
        d.openedAt = uint64(block.timestamp);
        emit DisputeOpened(jobId, msg.sender);
    }

    /// @notice Worker-only. Submit an evidence pointer during the dispute window.
    ///         Caller must match the `worker` recorded in `BountyMarket.getJob(id)`.
    function submitEvidence(uint256 jobId, bytes32 pointer) external {
        Dispute storage d = _disputes[jobId];
        if (d.state != DisputeState.Pending) revert WrongState();

        BountyMarket.Job memory job = market.getJob(jobId);
        if (job.worker != msg.sender) revert NotWorker();

        d.state = DisputeState.Responded;
        d.evidencePointer = pointer;
        emit EvidenceSubmitted(jobId, msg.sender, pointer);
    }

    /// @notice Mediator adjudicates a Pending or Responded dispute. Forwards the
    ///         final outcome to `BountyMarket.resolve`.
    function finalizeDispute(uint256 jobId, bool outcome) external onlyMediator {
        Dispute storage d = _disputes[jobId];
        if (d.state != DisputeState.Pending && d.state != DisputeState.Responded) {
            revert WrongState();
        }
        d.state = DisputeState.Adjudicated;
        d.finalOutcome = outcome;
        market.resolve(jobId, outcome);
        emit DisputeFinalized(jobId, outcome, msg.sender);
    }

    /// @notice Permissionless. Finalizes an expired dispute. Default outcome:
    ///         - Pending (no response) → reject (uphold the mediator)
    ///         - Responded (worker submitted evidence) → accept (reward the response)
    function finalizeExpired(uint256 jobId) external {
        Dispute storage d = _disputes[jobId];
        if (d.state != DisputeState.Pending && d.state != DisputeState.Responded) {
            revert WrongState();
        }
        if (block.timestamp < d.openedAt + EVIDENCE_WINDOW) revert WindowNotExpired();

        bool outcome = (d.state == DisputeState.Responded);
        d.state = DisputeState.Adjudicated;
        d.finalOutcome = outcome;
        market.resolve(jobId, outcome);
        emit DisputeFinalized(jobId, outcome, msg.sender);
    }

    function getDispute(uint256 jobId) external view returns (Dispute memory) {
        return _disputes[jobId];
    }
}
