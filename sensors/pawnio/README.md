# PawnIO modules

These are the official, signed PawnIO modules used by the OpenLinkHub Sensors service, taken
unmodified from PawnIO.Modules release 0.2.11 (`release_0_2_11.zip`, sha256
`43608cb89bc84247fef1368a139013f7d043e17db6d6c8dfc9b46bf0905a81f4`).

| File | Used for | sha256 |
|---|---|---|
| `AMDFamily17.bin` | AMD Ryzen / Threadripper / EPYC (families 17h–1Ah): reads the Tctl register | `dae74615761b78bdf064dfb3e136252ddcc6fc727d88f14738d0e5800d427a91` |
| `IntelMSR.bin` | Intel Core: reads the package thermal status MSR | `d6ed85d65ab17a22f813ef98207d6d537155ee2ded5976a21cb48413c9b92e5f` |

They are licensed under the GNU LGPL 2.1 (see `COPYING.LGPL-2.1`). The PawnIO driver itself is
not included; users install it separately (`winget install namazso.PawnIO`).
