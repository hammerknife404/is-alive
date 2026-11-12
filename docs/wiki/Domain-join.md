The tab has two halves, picked at its top: **This computer** and **Other computers**.

**This computer:** how this computer is joined, and to which domain: **Active Directory**, **Microsoft Entra ID** (Azure AD), both (**hybrid**), only **registered** with a work account, or a **workgroup**. On Windows it asks `dsregcmd /status`, as you would at a prompt; on Linux it asks realmd (`realm list`). It looks only at this computer.

It's checked when is-alive opens, and the answer is in the status bar as **Domain …**: `corp.example.com`, `corp.example.com + Entra` for a hybrid join, `Entra · tenant` for Entra ID only, `none` for a workgroup. Hover it for the details, click it for this tab. The status bar's **Refresh** checks again.

**▶ Check** asks again and logs a **Domain join** run to [[Results|Results]] (*hybrid · corp.example.com*); a **Full scan** logs one too. The tab shows the verdict, then **AD domain** (the DNS name), **Short name** (NetBIOS), **Entra tenant**, and the fields by section: the join flags, **DeviceId**, **DeviceAuthStatus**, **TpmProtected**, **TenantName** and **TenantId**, **MdmUrl** (Intune), **AzureAdPrt** (single sign-on to Microsoft 365), and the DRS, AD and DC diagnostic tests. YES and SUCCESS are green. **Everything** shows every field the tool printed, the signed-in account and diagnostics among them; Results keeps only the fields above, not the account, so a Site Bundle or Site report doesn't carry it. **Copy** and **Save…** take what's on screen. **Compare** in Results matches fields by name, so a join that broke since the last visit shows as a change.

On Linux only a realmd join (`realm join`, sssd) is seen: a computer joined with winbind or by hand reads as *Couldn't tell* without realmd, or as a workgroup with it.

**Other computers:** whether computers you name are in an Active Directory domain, asked from your laptop without an account. Type one or several in **Computers** (names or addresses, 32 at most; a range or subnet is refused) and press **▶ Start**. is-alive starts a sign-in on SMB (445) and stops at the server's first reply, an NTLM challenge that names the computer and its domain before any user name or password; it sends neither, so no account can be locked out. With 445 shut it does the same on RDP (3389) when the computer takes Network Level Authentication.

| Verdict | Meaning |
|---|---|
| **AD: corp.example.com** | It names a domain that isn't itself, with a DNS name: a member of that domain or one of its domain controllers |
| **Not AD-joined (Entra not checked)** | It names no domain but itself. An Entra ID join shows only to someone signed in, so it is never called a workgroup machine as a fact |
| **Answers, but doesn't say** | A port answered without names: SMB 1 only, NTLM off, RDP without NLA, or the name didn't resolve (hover for which) |
| **No answer on 445 or 3389** | Both shut or filtered: off, firewalled, or not a Windows computer |

A Map card's right-click **Open in a tab › Domain join** and a Subnet Sweep row's **Domain join** action fill the box with that address; nothing is sent until you press Start, and it is never part of a sweep or a Full scan. The run goes to [[Results|Results]] as **Domain join (others)** (*2 of 5 in Active Directory*). The computer asked logs an unfinished sign-in from your laptop's address.
