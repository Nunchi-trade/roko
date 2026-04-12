# Regulatory Compliance

> Regulatory frameworks, compliance standards, and legal precedents relevant to autonomous agent operation, financial services, and AI governance.

**Topic**: [References](./INDEX.md)
**Prerequisites**: [Architecture](../00-architecture/INDEX.md), [Harness](../03-harness/INDEX.md)
**Key sources**: `bardo-backup/prd/shared/citations.md` §§8, 12, `refactoring-prd/09-innovations.md` §IX

---

## Abstract

Autonomous agents operating in regulated domains (financial services, healthcare, enterprise) must comply with existing regulatory frameworks. Roko's Forensic AI innovation — content-addressed causal replay — provides the auditability that regulations require. This section collects the regulatory standards, compliance frameworks, and industry guidance relevant to agent operation.

---

## AI Governance

- European Union (2024). EU AI Act. Regulation (EU) 2024/1689.
  *Grounds: AI risk classification — the EU AI Act classifies AI systems by risk level and imposes obligations proportional to risk. High-risk AI systems (including those used in financial services) require transparency, human oversight, and robustness. Roko's Forensic AI provides the audit trail for compliance.*

---

## Financial Services Regulation

- U.S. Securities and Exchange Commission (SEC). Securities Exchange Act of 1934; Investment Advisers Act of 1940.
  *Grounds: Agent accountability — autonomous agents making investment decisions may trigger investment adviser regulations. Roko's content-addressed Engram DAG provides the audit trail for demonstrating compliance with fiduciary duties.*

- U.S. Commodity Futures Trading Commission (CFTC). Commodity Exchange Act.
  *Grounds: Derivatives compliance — agents operating with DeFi derivatives must consider CFTC jurisdiction. Forensic AI replay enables demonstrating that agent decisions followed programmatic rules.*

- European Union. MiFID II (Markets in Financial Instruments Directive). Directive 2014/65/EU.
  *Grounds: Algo trading — MiFID II requires firms using algorithmic trading to maintain records of order placement, including the algorithm's decision rationale. Roko's lineage DAG and episode logs satisfy this requirement.*

---

## Data Privacy

- European Union. GDPR (General Data Protection Regulation). Regulation (EU) 2016/679.
  *Grounds: Data handling — agents processing personal data must comply with GDPR's principles of data minimization, purpose limitation, and the right to be forgotten. Knowledge decay (Ebbinghaus half-life) provides a structural implementation of data minimization.*

- U.S. Congress. HIPAA (Health Insurance Portability and Accountability Act). 1996.
  *Grounds: Healthcare agents — agents operating in healthcare domains must comply with HIPAA's privacy and security rules. Roko's capability-based access control and permissioned subnets support domain-specific compliance.*

---

## Financial Reporting

- U.S. Congress. SOX (Sarbanes-Oxley Act). 2002.
  *Grounds: Audit trails — SOX requires adequate internal controls and audit trails for financial reporting. Roko's content-addressed Engram DAG and episode logs provide immutable audit trails for agent decision-making.*

---

## Content Provenance

- C2PA. Coalition for Content Provenance and Authenticity Standard. c2pa.org.
  *Grounds: Content provenance — industry standard for tracking content origin and modification history. Grounds the Attestation field on Engrams. Cross-referenced in [08-security-and-provenance.md](./08-security-and-provenance.md).*

---

## DeFi Compliance

- Zbandut, A. et al. (2025). Vault Disclosure Requirements for Agent-Operated DeFi Strategies. 2025.
  *Grounds: Agent disclosure — disclosure requirements for agent-operated DeFi vaults. Informs transparency requirements for chain domain agents.*

- Schrepel, T. (2024). The Trust Dilemma in Autonomous Agent Systems. _Stanford Law Review_.
  *Grounds: Trust framework — legal analysis of trust relationships in autonomous agent systems. Addresses the liability question: when an agent causes harm, who is responsible?*

---

## Cross-references

- See [08-security-and-provenance.md](./08-security-and-provenance.md) for security architecture
- See [22-protocol-standards.md](./22-protocol-standards.md) for ERC standards
