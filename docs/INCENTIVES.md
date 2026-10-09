# Non-Cryptocurrency Reciprocity & Incentive Architecture

**Objective:** Sustained participation incentives and fair resource balancing without speculative cryptocurrency or token volatility.

---

## 1. Credit Formulation

Reciprocity credits measure useful network contribution:

$$\text{Credits} = w_s \cdot S_{\text{verified}} + w_u \cdot U_{\text{uptime}} + w_b \cdot B_{\text{useful}} - P_{\text{penalties}}$$

- $S_{\text{verified}}$: Verified gigabyte-hours of stored shards (proven through Merkle audits).
- $U_{\text{uptime}}$: Normalized peer availability on network.
- $B_{\text{useful}}$: Useful bandwidth provided for peer shard retrieval.
- $P_{\text{penalties}}$: Deductions for failed audits or unannounced departures.

---

## 2. Utility & Benefits

1. **Storage Allowance**: Earned credits dictate how much cloud backup storage a user is permitted to upload across the mesh.
2. **Retrieval Priority**: High-credit peers receive priority bandwidth during congestion.
3. **Sybil Resistance**: New nodes start with zero credit and cannot immediately consume global storage without providing initial bounded contribution.
