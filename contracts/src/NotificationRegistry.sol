// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

/// @title NotificationRegistry — on-chain subscription preferences for event notifications.
/// @notice Subscribers (EOAs or contracts) register notification preferences keyed by
///         `msg.sender`. An off-chain indexer watches emitting contracts (e.g.
///         `BountyMarket`, `InsightBoard`) and dispatches to each subscriber's active
///         preferences via webhook / email / in-app. The indexer is out of scope for
///         this contract — this registry is pure storage + events.
/// @dev    Tombstone-on-remove: `removePreference` sets `active=false` and clears
///         `target` but does NOT shift the array. Indices remain stable for off-chain
///         consumers. Callers can reuse tombstone slots by overwriting via `setActive`
///         if desired, though the `target` will remain empty.
contract NotificationRegistry {
    enum Kind { Webhook, Email, InApp }

    struct Preference {
        Kind kind;
        string target;
        bool active;
    }

    mapping(address => Preference[]) private _preferences;

    event PreferenceAdded(address indexed subscriber, uint256 indexed index, Kind kind, string target);
    event PreferenceRemoved(address indexed subscriber, uint256 indexed index);
    event PreferenceActiveSet(address indexed subscriber, uint256 indexed index, bool active);

    error IndexOutOfBounds();
    error EmptyTarget();

    /// @notice Add a new notification preference for `msg.sender`. Returns the
    ///         assigned index (stable for the subscriber).
    function addPreference(Kind kind, string calldata target) external returns (uint256 index) {
        if (bytes(target).length == 0) revert EmptyTarget();
        _preferences[msg.sender].push(Preference({
            kind: kind,
            target: target,
            active: true
        }));
        index = _preferences[msg.sender].length - 1;
        emit PreferenceAdded(msg.sender, index, kind, target);
    }

    /// @notice Tombstone the preference at `index`. Does not shift the array — the
    ///         slot is retained with `active=false` and empty `target` so downstream
    ///         indices remain stable.
    function removePreference(uint256 index) external {
        Preference[] storage prefs = _preferences[msg.sender];
        if (index >= prefs.length) revert IndexOutOfBounds();
        prefs[index].active = false;
        prefs[index].target = "";
        emit PreferenceRemoved(msg.sender, index);
    }

    /// @notice Toggle `active` on the preference at `index`. Used to pause/resume
    ///         without losing the target string.
    function setActive(uint256 index, bool active) external {
        Preference[] storage prefs = _preferences[msg.sender];
        if (index >= prefs.length) revert IndexOutOfBounds();
        prefs[index].active = active;
        emit PreferenceActiveSet(msg.sender, index, active);
    }

    function getPreferences(address subscriber) external view returns (Preference[] memory) {
        return _preferences[subscriber];
    }

    function hasActivePreference(address subscriber) external view returns (bool) {
        Preference[] storage prefs = _preferences[subscriber];
        for (uint256 i = 0; i < prefs.length; i++) {
            if (prefs[i].active) return true;
        }
        return false;
    }

    function preferenceCount(address subscriber) external view returns (uint256) {
        return _preferences[subscriber].length;
    }
}
