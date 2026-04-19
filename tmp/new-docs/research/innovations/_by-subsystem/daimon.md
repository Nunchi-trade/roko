# Subsystem: Daimon

Innovations that interact with the Daimon subsystem — PAD affect vectors, somatic landscape, appraisal coefficients, and behavioural strategy selection.

| Slug | Interaction |
|---|---|
| [hdc-active-inference](../hdc-active-inference.md) | Daimon PAD baseline initialises the μ_prior belief vector; personality encodes prior beliefs. |
| [affect-causal-discovery](../affect-causal-discovery.md) | Extends Daimon with a Structural Causal Model over PAD; replaces fixed OCC/Scherer appraisal deltas with learned causal coefficients. |
| [code-somatic-markers](../code-somatic-markers.md) | Adds `CodeSomaticEngine` to Daimon; queries the existing somatic landscape k-d tree with code-derived 8D coordinates before task dispatch. |
