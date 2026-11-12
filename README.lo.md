<p align="center">
  <a href="https://getartcraft.com/">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="docs/brand/artcraft-logo-white.svg">
      <img alt="ArtCraft" src="docs/brand/artcraft-logo.svg" width="200">
    </picture>
  </a>
</p>

<p align="center">
  <img src="assets/app-icon/hicolor/256x256/apps/ai.storyteller.craft-anything.png" width="128" height="128" alt="ໄອຄອນ Craft Anything: ຮູບນົກກາທີ່ສະຫຼັກຢູ່ເທິງແຜ່ນກະເບື້ອງສີຟ້າ">
</p>

<h1 align="center">Craft Anything</h1>

<p align="center">
  <a href="README.md">English</a> · <a href="README.ja.md">日本語</a> · <a href="README.zh.md">简体中文</a> · <a href="README.zh-hant.md">繁體中文</a> · <a href="README.ko.md">한국어</a> · <a href="README.es.md">Español</a> · <a href="README.fr.md">Français</a> · <a href="README.vi.md">Tiếng Việt</a> · <a href="README.pt.md">Português</a> · <a href="README.th.md">ไทย</a> · <b>ພາສາລາວ</b>
</p>

<p align="center">
  <b>ເຂົ້າໃຈການເຮັດວຽກຂອງທຸກເວັບໄຊ, ແອັບ, ຫຼື ເກມ, ແລ້ວສ້າງເວີຊັນຂອງທ່ານເອງດ້ວຍ AI. ເຄື່ອງຈັກ clean-room ແບບໂອເພນຊອດທີ່ສ້າງດ້ວຍ Rust ລ້ວນໆ.</b><br>
  ພຽງແຕ່ກຳນົດເປົ້າໝາຍ ແລະ ບອກສິ່ງທີ່ຕ້ອງການ, ລະບົບຈະວິເຄາະຕົ້ນສະບັບດ້ວຍເຄື່ອງມືກວດສອບຕົວຈິງ<br>
  ຈາກນັ້ນຕົວແທນຂຽນໂຄດຈະສ້າງໂຄງການໃໝ່, ພ້ອມທັງປຽບທຽບກັບຕົ້ນສະບັບເພື່ອປັບປຸງຜົນລັບໃຫ້ສົມບູນ.
</p>

<p align="center">
  <img alt="100% Rust" src="https://img.shields.io/badge/100%25-Rust-b7410e?style=flat-square&logo=rust">
  <img alt="macOS · Windows · Linux · FreeBSD · Web" src="https://img.shields.io/badge/macOS%20%C2%B7%20Windows%20%C2%B7%20Linux%20%C2%B7%20FreeBSD%20%C2%B7%20Web-native-2d81ff?style=flat-square">
  <img alt="License: MIT OR Apache-2.0" src="https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-3a3a3a?style=flat-square">
  <img alt="Status: early alpha" src="https://img.shields.io/badge/status-early%20alpha-d69e2e?style=flat-square">
</p>

<p align="center">
  <a href="https://discord.gg/artcraft"><img alt="ເຂົ້າຮ່ວມ Discord ຂອງຊຸມຊົນ ArtCraft" src="https://img.shields.io/badge/Join%20us%20on%20Discord-5865F2?style=for-the-badge&logo=discord&logoColor=white" height="40"></a>
</p>

<p align="center">
  <a href="#downloads"><b>ດາວໂຫຼດ Craft Anything</b></a> ·
  <a href="https://getartcraft.com/">ArtCraft</a> ·
  <a href="https://getartcraft.com/apps">ແອັບ Crafting ທັງໝົດ</a>
</p>

<br>

<p align="center">
  <img src="docs/images/project-website.png" alt="ໂຄງການເວັບໄຊໃນ Craft Anything: ການກວດສອບສະແດງໃຫ້ເຫັນສ່ວນທີ່ຍັງບໍ່ກົງກັນ ແລະ ຄວາມຄືກັນຂອງແຕ່ລະໜ້າ, ຕົວສ້າງແຈ້ງວ່າເວີຊັນຂອງຕົນພ້ອມແລ້ວ, ບັດອະນຸມັດຖາມກ່ອນການດຳເນີນຄຳສັ່ງ, ແລະ ແຜນງານສະແດງວ່າສຳເລັດສອງຈາກສີ່ຂັ້ນຕອນແລ້ວ" width="100%">
  <br>
  <sub>ໂຄງການເວັບໄຊ: ການກວດສອບແຕ່ລະຄັ້ງຈະສະແດງໃຫ້ເຫັນວ່າການສ້າງຄືກັບຕົ້ນສະບັບຫຼາຍປານໃດ, ວຽກທີ່ຢູ່ນອກກົດຕ້ອງໄດ້ຮັບການອະນຸມັດດ້ວຍຕົນເອງ, ແລະ ແຜນງານຊ່ວຍໃຫ້ເຫັນວຽກທີ່ຍັງເຫຼືອ.</sub>
</p>

> [!NOTE]
> **ArtCraft ແມ່ນຊຸມຊົນຂອງສິລະປິນຈາກຫຼາຍຂະແໜງການ.** ນັກແຕ້ມຮູບ, ຊ່າງຖ່າຍຮູບ,
> ຜູ້ສ້າງຮູບເງົາ, ນັກແຕ້ມພາບປະກອບ, ນັກອອກແບບ, ນັກສ້າງແອນິເມຊັນ, ຜູ້ສ້າງສັນເປັນວຽກອະດິເລກ, ແລະ ຜູ້ທີ່ຫາກໍ່
> ຈັບສໍເປັນຄັ້ງທຳອິດໃນອາທິດແລ້ວນີ້. ຖ້າທ່ານສ້າງສິ່ງໃດໜຶ່ງຂຶ້ນມາ, ທ່ານກໍແມ່ນໜຶ່ງໃນພວກເຮົາ. **[ມາທັກທາຍກັນໄດ້ທີ່ Discord](https://discord.gg/artcraft).**

<p align="center">
  <a href="#how-it-works">ວິທີການເຮັດວຽກ</a> ·
  <a href="#two-ways-to-use-it">ສອງວິທີໃນການນຳໃຊ້</a> ·
  <a href="#ai">AI</a> ·
  <a href="#built-for-agents">ຕົວແທນ (Agents)</a> ·
  <a href="#under-the-hood">ໂຄງສ້າງພາຍໃນ</a> ·
  <a href="#get-started">ເລີ່ມຕົ້ນນຳໃຊ້</a> ·
  <a href="#status">ສະຖານະປະຈຸບັນ</a> ·
  <a href="#downloads">ດາວໂຫຼດ</a> ·
  <a href="https://discord.gg/artcraft">Discord</a>
</p>

<br>

<table>
  <tr>
    <td width="25%" valign="top">
      <h3><a id="-study-anything"></a>🔍 ສຶກສາວິເຄາະທຸກຢ່າງ</h3>
      ເວັບໄຊ, ແອັບເດັສທັອບ, ແອັບມືຖື, ເກມ, ໄຟລ໌: ເຄື່ອງມືກວດສອບຕົວຈິງຈະຄົ້ນຫາວ່າສິ່ງນັ້ນຖືກສ້າງຂຶ້ນມາແນວໃດ ແລະ ເຮັດວຽກຫຍັງ, ພ້ອມທັງສະແດງຫຼັກຖານປະກອບທຸກຄຳຕອບ.
    </td>
    <td width="25%" valign="top">
      <h3><a id="-build-your-own"></a>🛠️ ສ້າງເວີຊັນຂອງທ່ານເອງ</h3>
      ພຽງແຕ່ບອກສິ່ງທີ່ທ່ານຕ້ອງການ ເຊັ່ນ "ສ້າງເວັບໄຊນີ້ຄືນໃໝ່", "ສ້າງລະບົບການຫຼິ້ນຂອງເກມນີ້ຄືນໃໝ່ດ້ວຍ Rust", ຕົວແທນຂຽນໂຄດຈະສ້າງໂຄງການໃໝ່ຕາມສະແຕັກທີ່ທ່ານຮ້ອງຂໍ.
    </td>
    <td width="25%" valign="top">
      <h3><a id="-checked-not-guessed"></a>✅ ກວດສອບຕົວຈິງ ບໍ່ແມ່ນການຄາດເດົາ</h3>
      ທຸກໆການສ້າງຈະຖືກປຽບທຽບກັບຕົ້ນສະບັບ (ໜ້າເວັບໃນສາມຂະໜາດຄວາມກວ້າງໜ້າຈໍ, ການທົດສອບພາຍໃນຂອງເກມ, ຜົນການເຮັດວຽກຂອງແອັບ), ແລະ ປັບແຕ່ງຈົນກວ່າຈະຄືກັນພຽງພໍ.
    </td>
    <td width="25%" valign="top">
      <h3><a id="-agent-ready"></a>🤖 ພ້ອມຮອງຮັບຕົວແທນ</h3>
      ເຄື່ອງມືທັງ 314 ຢ່າງລ້ວນແຕ່ເປັນຄຳສັ່ງ CLI ແລະ ເຄື່ອງມື MCP, ດັ່ງນັ້ນຕົວແທນໃດໆກໍສາມາດຄວບຄຸມເຄື່ອງຈັກດຽວກັນກັບທີ່ແອັບໃຊ້. ບໍ່ຈຳເປັນຕ້ອງມີ AI ໃນການກວດກາ.
    </td>
  </tr>
</table>

<br>

## <a id="how-it-works"></a>ວິທີການເຮັດວຽກ

ໃນ Craft Anything, ໃຫ້ກຳນົດເປົ້າໝາຍ ແລະ ບອກສິ່ງທີ່ຕ້ອງການ ແລ້ວກົດ Start. ລະບົບຈະດຳເນີນການຜ່ານສີ່ຂັ້ນຕອນ,
ເມື່ອຕ້ອງການການຕັດສິນໃຈ, ລະບົບຈະຖາມໂດຍກົງດ້ວຍພາສາທີ່ເຂົ້າໃຈງ່າຍ:

1. **Study:** ກວດກາເບິ່ງຕົ້ນສະບັບ. ກວດເບິ່ງໜ້າເວັບຕາມຄວາມກວ້າງໜ້າຈໍ, ໂຄດ, ໄຟລ໌, ແລະ ວິທີການເຮັດວຽກ.
2. **Build:** ຕົວແທນຂຽນໂຄດຈະສ້າງໂຄງການໃໝ່. ໂດຍໃຊ້ພາສາ, ເຟຣມເວີກ, ຫຼື ເຄື່ອງຈັກທີ່ລະບຸໄວ້ໃນຄຳຂໍ, ຖ້າບໍ່ໄດ້ລະບຸຈະໃຊ້ຄ່າເລີ່ມຕົ້ນ.
3. **Check:** ປຽບທຽບສິ່ງທີ່ສ້າງຂຶ້ນກັບຕົ້ນສະບັບ, ແລະ ສະແດງລາຍການສ່ວນທີ່ຍັງບໍ່ກົງກັນ.
4. **Ready:** ສາມາດເປີດ, ຫຼິ້ນ, ຫຼື ເບິ່ງຕົວຢ່າງຜົນລັບໄດ້. ຫາກຕ້ອງການປ່ຽນແປງສ່ວນໃດກໍສາມາດຮ້ອງຂໍເພື່ອປັບປຸງໄດ້.

<table>
  <tr>
    <td width="50%" valign="top">
      <img src="docs/images/make.png" alt="ໜ້າຈໍ Make: ເລືອກເວັບໄຊ, ແອັບ ຫຼື ເກມທີ່ຈະສ້າງຄືນໃໝ່, ປ້ອນທີ່ຢູ່, ເລືອກລະຫວ່າງ Clean room ຫຼື Deep, ບອກສິ່ງທີ່ຕ້ອງການ ແລ້ວກົດ Start" width="100%"><br>
      <sub><b>Make:</b> ເລືອກເປົ້າໝາຍທີ່ຈະສ້າງຄືນໃໝ່, ບອກສິ່ງທີ່ຕ້ອງການ ແລ້ວກົດ Start.</sub>
    </td>
    <td width="50%" valign="top">
      <img src="docs/images/project-game.png" alt="ໂຄງການເກມງູທີ່ສຳເລັດທຸກຂັ້ນຕອນແລ້ວ ພ້ອມປຸ່ມຫຼິ້ນເກມ" width="100%"><br>
      <sub><b>Ready:</b> ຫຼິ້ນເກມ ຫຼື ຮ້ອງຂໍການປ່ຽນແປງໄດ້ຢູ່ດ້ານລຸ່ມ.</sub>
    </td>
  </tr>
</table>

## <a id="two-ways-to-use-it"></a>ສອງວິທີໃນການນຳໃຊ້

- **Simple** (ຄ່າເລີ່ມຕົ້ນ): "What do you want to make?" ເລືອກເວັບໄຊ, ແອັບ, ຫຼື ເກມ ແລ້ວອະທິບາຍສິ່ງທີ່ຕ້ອງການ ຈາກນັ້ນກົດ Start.
  ໜ້າໂຄງການຈະສະແດງສີ່ຂັ້ນຕອນ (Study → Build → Check → Ready), ລາຍລະອຽດສິ່ງທີ່ຕົວແທນກຳລັງເຮັດດ້ວຍຄຳອະທິບາຍທີ່ເຂົ້າໃຈງ່າຍ,
  ບັດຄຳຖາມ ແລະ ການອະນຸມັດ, ລະດັບຄວາມຄືກັນລະຫວ່າງຜົນລັບກັບຕົ້ນສະບັບ, ແລະ ປຸ່ມສຳລັບເປີດຜົນລັບ ຫຼື ເປີດໂຟນເດີ.
  ສາມາດຮ້ອງຂໍການປ່ຽນແປງໄດ້ຈາກຊ່ອງປ້ອນຂໍ້ມູນດ້ານລຸ່ມ. ສ່ວນຕົວເລືອກ "Just look inside" ຈະບໍ່ສ້າງຜົນລັບ, ແຕ່ຈະເປີດເປົ້າໝາຍເພື່ອຖາມຄຳຖາມເທົ່ານັ້ນ (ໃນໂໝດ Deep).
- **Advanced**: ໂຕະເຮັດວຽກ (Workbench) ທີ່ມີຟັງຊັນຄົບຖ້ວນ. ປະກອບມີເຄື່ອງມືທັງໝົດ (Ctrl/Cmd+K), ຟັງຊັນ, ສາຍອັກຂະຫຼະ, ໂຄດທຽມ C, ດິສແອັດແຊມບລີ,
  ເລກຖານສິບຫົກ (Hex), ກຣາຟການເອີ້ນໃຊ້, ຫຼັກຖານ, ສິ່ງທີ່ຍັງບໍ່ທັນຢືນຢັນ, ໜ້າວຽກ, ບັນທຶກ (Logs), ແລະ ຄອນໂຊລຕົວແທນ.

ທັງສອງໂໝດລ້ວນແຕ່ໃຊ້ເຄື່ອງຈັກດຽວກັນ.

## <a id="clean-room-or-deep"></a>Clean room ຫຼື Deep

- **Clean room** (ຄ່າເລີ່ມຕົ້ນ): ຈະບໍ່ອ່ານສິ່ງທີ່ຢູ່ພາຍໃນຕົ້ນສະບັບເລີຍ ເຊັ່ນ ໂຄດ, ໄຟລ໌, ຫຼື ຊອດໂຄດຂອງໜ້າເວັບ. Craft Anything
  ຈະເຮັດວຽກສະເພາະກັບສິ່ງທີ່ເບິ່ງເຫັນດ້ວຍຕາເທົ່ານັ້ນ. ເຊິ່ງລວມເຖິງຮູບພາບທີ່ຜູ້ໃຊ້ເພີ່ມເຂົ້າໄປ, ເວັບໄຊທີ່ຜູ້ເຂົ້າຊົມເຫັນ (ຮູບຖ່າຍໜ້າຈໍ, ຂໍ້ຄວາມທີ່ເຫັນໄດ້,
  ໂຄງຮ່າງທີ່ວັດແທກໄດ້, ສີສັນ, ຟອນຕົວໜັງສື, ໄລຍະຫ່າງ, ເມນູທີ່ເປີດເບິ່ງ), ແລະ ໜ້າຈໍແອັບທີ່ບັນທຶກໄວ້ໃນກໍລະນີທີ່ໄດ້ຮັບອະນຸຍາດ.
  ເຄື່ອງມືທີ່ໃຊ້ສ່ອງເບິ່ງພາຍໃນຈະຖືກປະຕິເສດ, ພ້ອມທັງແນະນຳວິທີການເປີດໃຊ້ໂໝດ Deep.
- **Deep**: ໃຊ້ງານເຄື່ອງມືທັງໝົດສຳລັບການນຳໃຊ້ສ່ວນຕົວ. ໂຄດ ແລະ ໄຟລ໌ຂອງໂປຣແກຣມຈະຖືກອ່ານດ້ວຍເຄື່ອງຈັກພາຍໃນ (ລວມທັງ Ghidra ຖ້າມີການຕິດຕັ້ງ),
  ອ່ານ JavaScript ຂອງເວັບໄຊ ແລະ ສັງເກດເບິ່ງໜ້າເລີ່ມຕົ້ນ, ໂດຍທີ່ຕົວສ້າງສາມາດນຳໃຊ້ສິ່ງເຫຼົ່ານີ້ໄດ້ທັງໝົດ.

ສາມາດສະຫຼັບໂໝດໄດ້ທີ່ແຖບຫົວຂໍ້ ຫຼື ໃນ Settings, ຫຼື ປ່ຽນດ້ວຍ `settings_set deep_mode true`, ຫຼື ຕື່ມ
`--deep` (ຫຼື `--clean-room`) ເມື່ອຮັນຄຳສັ່ງ CLI ແຕ່ລະຄັ້ງ.

<p align="center">
  <img src="docs/images/workbench.png" alt="Advanced Workbench: ລາຍຊື່ຟັງຊັນ, ໂຄດທຽມ C ຂອງຟັງຊັນທີ່ເລືອກ, ຕົວແທນຕອບຄຳຖາມຜ່ານການເອີ້ນໃຊ້ເຄື່ອງມື, ບັນທຶກຫຼັກຖານ" width="100%">
  <br>
  <sub><b>Advanced:</b> ຟັງຊັນ, ໂຄດທຽມ C, ຕົວແທນ, ແລະ ຫຼັກຖານທີ່ຢູ່ເບື້ອງຫຼັງທຸກໆຄຳຕອບ.</sub>
</p>

## <a id="ai"></a>AI

ການສ້າງໂຄງການຮຽກຮ້ອງໃຫ້ມີຕົວແທນຂຽນໂຄດຢູ່ໃນເຄື່ອງຄອມພິວເຕີຂອງທ່ານ. ລະບົບຮອງຮັບ **Claude Code** ເປັນອັນດັບທຳອິດ. Craft Anything ຈະຮັນໂປຣແກຣມ `claude` ແທ້ທີ່ຕິດຕັ້ງຢູ່ໃນເຄື່ອງໂດຍກົງໂດຍບໍ່ມີການດັດແປງໃດໆ, ແລະ ໃຊ້ງານບັນຊີທີ່ເຂົ້າສູ່ລະບົບໄວ້ແລ້ວຂອງທ່ານ. Craft Anything ຈະບໍ່ເຂົ້າເບິ່ງ ຫຼື ຈັດການບັນຊີຂອງທ່ານ. ການນຳໃຊ້ນີ້ຢູ່ພາຍໃຕ້ເງື່ອນໄຂຂໍ້ຕົກລົງລະຫວ່າງທ່ານກັບ Anthropic. ສຳລັບວິທີການ ແລະ ເຫດຜົນ, ເບິ່ງໄດ້ທີ່ [docs/claude-code.md](docs/claude-code.md). ນອກຈາກນີ້ຍັງມີຕົວເຊື່ອມຕໍ່ Codex, ແຕ່ຍັງບໍ່ທັນໄດ້ຮັບການທົດສອບໃນສະພາບແວດລ້ອມການຕິດຕັ້ງຕົວຈິງ. ສຳລັບການຕັ້ງຄຳຖາມ ແລະ ການວິເຄາະ, ທ່ານສາມາດໃຊ້ Anthropic, OpenAI, OpenRouter, ແລະ ເຊີເວີພາຍໃນທີ່ຮອງຮັບຮູບແບບ OpenAI ໄດ້ (Settings → AI).

## <a id="built-for-agents"></a>ອອກແບບມາເພື່ອຕົວແທນ

ທຸກໆເຄື່ອງມືລ້ວນແຕ່ເປັນຄຳສັ່ງຍ່ອຍຂອງ CLI ແລະ ເຄື່ອງມື MCP, ດັ່ງນັ້ນຕົວແທນຈຶ່ງສາມາດສຳຫຼວດເປົ້າໝາຍ ຫຼື ດຳເນີນການສ້າງໃໝ່ທັງໝົດໄດ້ໂດຍບໍ່ຕ້ອງເປີດແອັບ:

```sh
craft-anything-cli commands                                   # every tool, by toolset
craft-anything-cli mcp                                        # serve them to any MCP client
craft-anything-cli rebuild https://example.com "Recreate this website"
```

ຜົນລັບແມ່ນບັນທຶກຫຼັກຖານທີ່ລະບຸລະດັບຄວາມໝັ້ນໃຈ, ຂອບເຂດ, ແລະ ຂໍ້ຈຳກັດ. ສຳລັບຜົນລັບຂະໜາດໃຫຍ່ຈະສະແດງບົດສະຫຼຸບກ່ອນ, ແລະ ສາມາດເບິ່ງແຍກເປັນໜ້າໄດ້ດ້ວຍ `core_evidence_browse`. ຄຳສັ່ງ `craft-anything-cli setup` ຈະເຊື່ອມຕໍ່ Craft Anything ເຂົ້າກັບແອັບຕົວແທນຂຽນໂຄດ.

## <a id="under-the-hood"></a>ໂຄງສ້າງພາຍໃນ

ເຄື່ອງຈັກວິສະວະກຳຍ້ອນຮອຍດ້ວຍ Rust ລ້ວນໆ ພ້ອມເຄື່ອງມື 314 ຢ່າງ (298 ຢ່າງໃນເຄື່ອງຈັກ, ສ່ວນທີ່ເຫຼືອແມ່ນມາຈາກຊັ້ນ AI ແລະ ການຕັ້ງຄ່າ) ສຳລັບໂປຣແກຣມເນທີບ, .NET, Java, Android, Apple, Electron/JavaScript, ເວັບໄຊ, ເຟີມແວ, ການດັກຈັບແພັກເກັດ, ຖານຂໍ້ມູນ, ແລະ ໄຟລ໌ເກມ. ຕົວຖອດລະຫັດ, ດິສແອັດແຊມເບຼີ, ແລະ ດີຄອມໄພເລີໃນຕົວສາມາດເຮັດວຽກໄດ້ໂດຍບໍ່ຕ້ອງອາໄສໂປຣແກຣມພາຍນອກ. ໂໝດ Deep ຈະໃຊ້ງານເຄື່ອງມືທັງໝົດນີ້ ແລະ ໃຊ້ Ghidra ນຳອີກຫາກມີການຕິດຕັ້ງໄວ້. ລະບົບນີ້ຂັບເຄື່ອນແອັບເດັສທັອບ (egui), CLI, ແລະ ເຊີເວີ MCP, ໂດຍບາງສ່ວນສາມາດເຮັດວຽກເທິງບຣາວເຊີຜ່ານ WebAssembly ໄດ້. ເບິ່ງລາຍການທັງໝົດໄດ້ທີ່ [docs/tools.md](docs/tools.md).

## <a id="get-started"></a>ເລີ່ມຕົ້ນນຳໃຊ້

ຕ້ອງການ **Rust 1.97 ຫຼື ໃໝ່ກວ່າ** (ຈຳເປັນສຳລັບແພັກເກັດທີ່ຂຶ້ນຕໍ່ກັນ, ອັບເດດໄດ້ດ້ວຍ `rustup update`). ສຳລັບ Debian ຫຼື Ubuntu, ແອັບເດັສທັອບຍັງຕ້ອງການແພັກເກັດຕໍ່ໄປນີ້ອີກດ້ວຍ:

```sh
sudo apt-get install libxkbcommon-dev libwayland-dev libx11-dev libxrandr-dev libxi-dev libgl1-mesa-dev libgtk-3-dev
```

ຈາກນັ້ນ:

```sh
git clone https://github.com/storytold/craft-anything && cd craft-anything
cargo run --release -p craft-anything                # the desktop app
cargo install --path apps/craft-anything-cli         # the CLI, as `craft-anything-cli`
craft-anything-cli doctor                            # checks the install, coding agents and engines
```

ການສ້າງໂຄງການຮຽກຮ້ອງໃຫ້ມີຕົວແທນຂຽນໂຄດທີ່ເຂົ້າສູ່ລະບົບໄວ້ໃນເຄື່ອງຄອມພິວເຕີຂອງທ່ານ (ເບິ່ງ [AI](#ai)). ຄຳສັ່ງ `doctor` ຈະແຈ້ງໃຫ້ຊາບວ່າມີຫຍັງຂາດຫາຍໄປ. ສ້າງຄືນໃໝ່ຜ່ານທາງແຖວຄຳສັ່ງ (ຄຳຖາມຈະສະແດງຂຶ້ນໃນເທີມິນອນ):

```sh
craft-anything-cli rebuild https://example.com "Recreate this website"
craft-anything-cli rebuild - "A snake game in Rust" --rounds 1
craft-anything-cli rebuild https://example.com "Make the header blue" --continue-in "<project folder>"
craft-anything-cli rebuild - "Add a contact form" --project-folder "<your folder>" --existing edit
```

ໂຄງການໃໝ່ຈະຖືກບັນທຶກໄວ້ໃນ `Documents/Craft Anything` (ສາມາດປ່ຽນໄດ້ໃນ Settings → General). ແຕ່ລະໂຟນເດີໂຄງການຈະປະກອບມີ `craft.json` (ຄຳຮ້ອງຂໍ, ເປົ້າໝາຍ, ສະຖານະ, ປະຫວັດ), `capture/` (ສິ່ງທີ່ບັນທຶກຈາກເປົ້າໝາຍ), `app/` (ໂຄງການທີ່ສ້າງຂຶ້ນ), `checks/` (ການປຽບທຽບແຕ່ລະຄັ້ງກັບຕົ້ນສະບັບ), `investigation.cany/` (ຫຼັກຖານ), ແລະ `inbox/` (ຄຳຖາມທີ່ຍັງບໍ່ທັນຕອບ).

ທ່ານຍັງສາມາດເລີ່ມໂຄງການຈາກໂຟນເດີທີ່ເລືອກເອງໄດ້ (ກົດ "Choose a folder…" ຢູ່ກ້ອງ Start ຫຼື ໃຊ້ `--project-folder`). ຖ້າຫາກໃນໂຟນເດີມີໄຟລ໌ຢູ່ແລ້ວ ພຽງແຕ່ລະບຸວ່າໄຟລ໌ເຫຼົ່ານັ້ນແມ່ນຫຍັງ. ທ່ານສາມາດເລືອກໄດ້ວ່າ ແມ່ນຕົວແອັບເອງທີ່ຕົວສ້າງຈະແກ້ໄຂໂດຍກົງຢູ່ບ່ອນນັ້ນເລີຍ, ຫຼື ແມ່ນເອກະສານອ້າງອີງສຳລັບໃຊ້ເປັນຈຸດເລີ່ມຕົ້ນ (ໃນກໍລະນີນີ້ ແອັບຈະຖືກສ້າງຂຶ້ນໃນໂຟນເດີ `app/` ທີ່ຢູ່ຂ້າງກັນ) (ສາມາດຕັ້ງຄ່າໃຫ້ຈື່ການເລືອກໄດ້ໃນ Settings → General). ໃນ Settings → General ຍັງຈະສະແດງລາຍຊື່ໂຄງການຈາກໂຟນເດີອື່ນໆອີກດ້ວຍ, ເຊິ່ງການລຶບອອກຈາກລາຍຊື່ຈະເປັນພຽງການເອົາອອກຈາກລາຍການເທົ່ານັ້ນ ໂດຍບໍ່ໄດ້ລຶບໄຟລ໌ຕົວຈິງຖິ້ມ.

## <a id="status"></a>ສະຖານະປະຈຸບັນ

> [!IMPORTANT]
> **ສະຖານະປະຈຸບັນ:** Craft Anything ຢູ່ໃນໄລຍະ Early Alpha. ສິ່ງທີ່ໃຊ້ໄດ້ແລ້ວ, ສິ່ງທີ່ໃຊ້ໄດ້ບາງສ່ວນ, ແລະ ແຜນງານຕໍ່ໄປມີດັ່ງນີ້:
>
> - **ສິ່ງທີ່ໃຊ້ໄດ້ແລ້ວ:** ແອັບເດັສທັອບ (Simple ແລະ Advanced), CLI, ເຊີເວີ MCP. ຂະບວນການເຮັດວຽກ Study → Build → Check → Refine ສຳລັບເວັບໄຊ, ແອັບ, ແລະ ເກມ ໂດຍໃຊ້ Claude Code ເປັນຕົວສ້າງ. ການກວດສອບເວັບໄຊໃນສາມຂະໜາດຄວາມກວ້າງໜ້າຈໍ. ການສ້າງເກມ Unity, Unreal ຫຼື Godot ຄືນໃໝ່. ໂໝດ Deep ທີ່ໃຊ້ງານ Ghidra. ການສ້າງຮູບພາບ ແລະ ໂມເດວ 3D (ເລືອກໄດ້, ໂດຍໃຊ້ຄີ fal.ai ຂອງທ່ານເອງ). ປລັກອິນ (MCP server, ແລະ ຄອມໂພເນນ WASM ເມື່ອສ້າງດ້ວຍຕົວເລືອກ `wasm-plugins`).
> - **ໃຊ້ໄດ້ບາງສ່ວນ:** ການສ້າງດ້ວຍ API key ແລະ ຕົວແທນຂຽນໂຄດອື່ນໆ (ມີການພັດທະນາແລ້ວ ແຕ່ຍັງບໍ່ທັນໄດ້ທົດລອງຮັນຕົວຈິງ), ການກວດສອບການເຮັດວຽກຂອງແອັບ ແລະ ເກມ, ເຄື່ອງມືເຄື່ອງຈັກບາງອັນທີ່ທົດສອບກັບເປົ້າໝາຍຈຳລອງເທົ່ານັ້ນ.
> - **ແຜນງານຕໍ່ໄປ:** ການປ່ອຍເວີຊັນທຳອິດສູ່ສາທາລະນະ (ໂປຣແກຣມຕິດຕັ້ງສຳລັບ Windows, macOS, Linux, FreeBSD ແລະ ຕົວສະແດງຜົນເທິງບຣາວເຊີພ້ອມແລ້ວໃນ CI), ຈາກນັ້ນແມ່ນແພັກເກດແບບຈັດການໃຫ້.
>
> ມີລາຍລະອຽດ ແລະ ວິທີການກວດສອບແຕ່ລະສ່ວນຢູ່ໃນ [docs/STATUS.md](docs/STATUS.md), ຕາຕະລາງກວດສອບແຍກຕາມເຄື່ອງມືໃນ [docs/tool-status.md](docs/tool-status.md), ແລະ ແຜນງານແຕ່ລະລາຍການໃນ [docs/ROADMAP.md](docs/ROADMAP.md). ສາມາດແຈ້ງບັນຫາ ຫຼື ແຈ້ງຈຸດທີ່ໃຊ້ການບໍ່ໄດ້ຜ່ານທາງ [Discord](https://discord.gg/artcraft).

**ການນຳໃຊ້ຢ່າງມີຄວາມຮັບຜິດຊອບ.** Craft Anything ແມ່ນໃຫ້ໃຊ້ສະເພາະກັບຊອບແວທີ່ທ່ານເປັນເຈົ້າຂອງ ຫຼື ມີສິດກວດກາເບິ່ງເທົ່ານັ້ນ. ລະບົບຈະບໍ່ຫຼີກລ່ຽງ DRM, ການເຂົ້າສູ່ລະບົບ, ຫຼື ລະບົບຕ້ານການໂກງຢ່າງເດັດຂາດ. ການສ້າງຄືນໃໝ່ຈະຈຳລອງໂຄງຮ່າງ, ຮູບແບບ, ແລະ ການເຮັດວຽກໂດຍອີງໃສ່ຫຼັກຖານ, ແລະ ຕົວແທນຈະໄດ້ຮັບຄຳສັ່ງບໍ່ໃຫ້ກ່າຍໂລໂກ້, ຮູບຖ່າຍ, ຫຼື ຂໍ້ຄວາມ ເວັ້ນເສຍແຕ່ຈະຢືນຢັນໄດ້ວ່າທ່ານເປັນເຈົ້າຂອງ ຫຼື ໄດ້ຮັບອະນຸຍາດແລ້ວ.

## <a id="downloads"></a>ດາວໂຫຼດ

ໃນ GitHub [ເວີຊັນປ່ອຍຫຼ້າສຸດ](https://github.com/storytold/craft-anything/releases/latest) ຈະມີທຸກໆບິລດ໌ຢູ່ດ້ານລຸ່ມນີ້, ແລະ [ເວີຊັນປ່ອຍທັງໝົດ](https://github.com/storytold/craft-anything/releases) ຈະມີເວີຊັນກ່ອນໜ້າພ້ອມທັງບັນທຶກປະຈຳເວີຊັນ. ສັນຍະລັກ `<ver>` ໃນຊື່ໄຟລ໌ແມ່ນໝາຍເລກເວີຊັນ, ແລະ `SHA256SUMS.txt` ມີຄ່າເຊັກຊຳຂອງທຸກໆໄຟລ໌.

### <a id="windows"></a>Windows

| ບິລດ໌                              | ໂປຣແກຣມຕິດຕັ້ງ                            | ແບບພົກພາ                                            |
| --------------------------------- | ---------------------------------------- | ------------------------------------------------- |
| x64 (Intel/AMD 64 ບິດ)            | `craft-anything-<ver>-windows-x64.msi`   | `craft-anything-<ver>-windows-x64-portable.zip`   |
| arm64 (Snapdragon ແລະ ARM PC ອື່ນໆ) | `craft-anything-<ver>-windows-arm64.msi` | `craft-anything-<ver>-windows-arm64-portable.zip` |
| x86 (32 ບິດ)                      | `craft-anything-<ver>-windows-x86.msi`   | `craft-anything-<ver>-windows-x86-portable.zip`   |

ໂປຣແກຣມຕິດຕັ້ງ ແລະ ໄຟລ໌ເອັກຊີຄິວທ໌ໄດ້ຮັບການເຊັນກຳກັບໂຄດແລ້ວ. ໄຟລ໌ zip ແບບພົກພາຈະເກັບຮັກສາການຕັ້ງຄ່າໄວ້ຂ້າງໆແອັບ.

### <a id="macos"></a>macOS

| ບິລດ໌                               | ໄຟລ໌                                           | ໝາຍເຫດ                                                   |
| ---------------------------------- | ---------------------------------------------- | ------------------------------------------------------ |
| ແອັບ, ຢູນິເວີຊັລ (Apple Silicon + Intel) | `craft-anything-<ver>-macos-universal.dmg`     | ເຊັນກຳກັບ ແລະ ຜ່ານການຮັບຮອງແລ້ວ                                         |
| ເຄື່ອງມືແຖວຄຳສັ່ງ, ຢູນິເວີຊັລ              | `craft-anything-cli-<ver>-macos-universal.zip` | ເຊັນກຳກັບ ແລະ ຜ່ານການຮັບຮອງແລ້ວ. ຖ້າ macOS ບລັອກ ໃຫ້ເບິ່ງດ້ານລຸ່ມ |

ເວີຊັນ 0.2.0 ເປັນຕົ້ນມາຈະມີເຄື່ອງມືແຖວຄຳສັ່ງລວມຢູ່ໃນແອັບແລ້ວ, ຈຶ່ງບໍ່ຈຳເປັນຕ້ອງມີໄຟລ໌ zip ແຍກຕ່າງຫາກເມື່ອສ້າງ. ໄຟລ໌ນີ້ຈຳເປັນສະເພາະເມື່ອຕ້ອງການໃຊ້ແຕ່ແຖວຄຳສັ່ງໂດຍບໍ່ມີແອັບເທົ່ານັ້ນ.

ຫາກດາວໂຫຼດໄຟລ໌ zip ຜ່ານບຣາວເຊີ ອາດຈະຕິດຂັດໂດຍມີຂໍ້ຄວາມແຈ້ງເຕືອນວ່າ "Apple could not verify craft-anything-cli" (ເນື່ອງຈາກໂປຣແກຣມດ່ຽວບໍ່ສາມາດແນບຂໍ້ມູນການຮັບຮອງໄປພ້ອມໄດ້, ເຮັດໃຫ້ macOS ຕ້ອງກວດສອບທາງອອນລາຍ). ຫາກດາວໂຫຼດຜ່ານເທີມິນອນຈະບໍ່ມີການກວດສອບນີ້, ດັ່ງນັ້ນໃຫ້ໃຊ້ຄຳສັ່ງຕໍ່ໄປນີ້ແທນ:

```sh
ver=0.2.0   # change this to the version you want
curl -fLO "https://github.com/storytold/craft-anything/releases/download/v$ver/craft-anything-cli-$ver-macos-universal.zip"
ditto -x -k "craft-anything-cli-$ver-macos-universal.zip" .
mkdir -p ~/.local/bin
mv "craft-anything-cli-$ver-macos-universal/craft-anything-cli" ~/.local/bin/
~/.local/bin/craft-anything-cli --version
```

ຫາກຕ້ອງການຮັນດ້ວຍຄຳສັ່ງ `craft-anything-cli` ໂດຍກົງ, ໃຫ້ເພີ່ມ `~/.local/bin` ເຂົ້າໄປໃນ `PATH`.

ຖ້າມີໄຟລ໌ທີ່ຖືກບລັອກຢູ່ແລ້ວ, ໃຫ້ກົດ Done ແທນ Move to Bin ແລ້ວຮັນຄຳສັ່ງ `xattr -d com.apple.quarantine <path to craft-anything-cli>`, ຫຼື ອະນຸຍາດໃນ System Settings → Privacy & Security → Open Anyway.

#### ການກວດສອບວ່າໄຟລ໌ດາວໂຫຼດສຳລັບ Mac ໄດ້ຮັບການເຊັນກຳກັບ ແລະ ຮັບຮອງແລ້ວຫຼືບໍ່

ເວີຊັນປ່ອຍສຳລັບ Mac ແມ່ນໄດ້ຮັບການເຊັນກຳກັບດ້ວຍໃບຢັ້ງຢືນ Developer ID ແລະ ຜ່ານການຮັບຮອງຈາກ Apple. ຫາກຕ້ອງການກວດສອບດ້ວຍຕົນເອງ, ໃຫ້ຮັນຄຳສັ່ງຕໍ່ໄປນີ້ໃນເທີມິນອນ (ປ່ຽນ `<ver>` ເປັນເວີຊັນ ແລະ ປ່ຽນເສັ້ນທາງໄຟລ໌ຕາມຕົວຈິງຂອງທ່ານ):

```sh
# ດິສກ໌ອິເມຈ: ຫາກຂຶ້ນ "accepted" ແລະ "source=Notarized Developer ID" ສະແດງວ່າຜ່ານ
spctl --assess --type open --context context:primary-signature -v "craft-anything-<ver>-macos-universal.dmg"
# ແນບປີ້ການຮັບຮອງແລ້ວ: ຈະສະແດງຂໍ້ຄວາມ "The validate action worked!"
xcrun stapler validate "craft-anything-<ver>-macos-universal.dmg"

# ແອັບ (ຫຼັງຈາກເປີດດິສກ໌ອິເມຈ): ລາຍເຊັນກຳກັບຕ້ອງໄດ້ຮັບການຢືນຢັນ
codesign --verify --deep --strict --verbose=2 "/Volumes/<disk image name>/Craft Anything.app"
spctl --assess --type execute --verbose "/Volumes/<disk image name>/Craft Anything.app"

# ເຄື່ອງມືແຖວຄຳສັ່ງ: ໂປຣແກຣມດ່ຽວບໍ່ສາມາດບັນຈຸປີ້ຮັບຮອງໄດ້ ສະນັ້ນ macOS ຈະກວດສອບທາງອອນລາຍ
codesign --verify --strict --verbose=2 ~/.local/bin/craft-anything-cli
codesign -dv --verbose=4 ~/.local/bin/craft-anything-cli 2>&1 | grep Authority
```

ໃນແຖວ `Authority` ຈະຕ້ອງສະແດງໃບຢັ້ງຢືນ "Developer ID Application". ຫາກໄຟລ໌ໃດບໍ່ຜ່ານການກວດສອບ ຢ່າຮັນໄຟລ໌ນັ້ນ, ແລະ ກະລຸນາແຈ້ງຜົນຂໍ້ຄວາມທີ່ສະແດງພ້ອມທັງແທັກເວີຊັນປ່ອຍໃຫ້ພວກເຮົາຊາບ.

### <a id="linux"></a>Linux

| ຮູບແບບ                 | x86_64                                       | aarch64 (ARM64)                               | ໝາຍເຫດ                                                                                                                |
| -------------------- | -------------------------------------------- | --------------------------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| AppImage             | `craft-anything-<ver>-linux-x86_64.AppImage` | `craft-anything-<ver>-linux-aarch64.AppImage` | ຮັນໄດ້ທຸກບ່ອນ. ອັບເດດໃນຕົວດ້ວຍ [AppImageUpdate](https://github.com/AppImageCommunity/AppImageUpdate) (ໄຟລ໌ `.zsync`) |
| Flatpak              | `craft-anything-<ver>-linux-x86_64.flatpak`  | `craft-anything-<ver>-linux-aarch64.flatpak`  | ແຊນບັອກ. `flatpak install --user <file>`                                                                           |
| Debian/Ubuntu        | `craft-anything-<ver>-linux-x86_64.deb`      | `craft-anything-<ver>-linux-aarch64.deb`      |                                                                                                                     |
| Fedora/RHEL/openSUSE | `craft-anything-<ver>-linux-x86_64.rpm`      | `craft-anything-<ver>-linux-aarch64.rpm`      |                                                                                                                     |
| Tarball              | `craft-anything-<ver>-linux-x86_64.tar.gz`   | `craft-anything-<ver>-linux-aarch64.tar.gz`   | ແຕກໄຟລ໌ໄວ້ບ່ອນໃດກໍໄດ້ຕາມຕ້ອງການ                                                                                               |

RISC-V (riscv64): `craft-anything-<ver>-linux-riscv64.tar.gz`, ໃຫ້ບໍລິການສະເພາະໄຟລ໌ tarball ເທົ່ານັ້ນ.

### <a id="freebsd"></a>FreeBSD

| ບິລດ໌   | ໄຟລ໌                                         |
| ------ | -------------------------------------------- |
| x86_64 | `craft-anything-<ver>-freebsd-x86_64.tar.gz` |

### <a id="web-webassembly"></a>Web (WebAssembly)

| ບິລດ໌          | ໄຟລ໌                           | ໝາຍເຫດ                                                                                                                                                                           |
| ------------- | ------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| ຕົວສະແດງຜົນເທິງບຣາວເຊີ | `craft-anything-web-<ver>.zip` | ວິເຄາະ ແລະ ບອກວ່າໄຟລ໌ແມ່ນຫຍັງພາຍໃນບຣາວເຊີ (ໄຟລ໌ຈະບໍ່ຖືກສົ່ງອອກນອກບຣາວເຊີ). ອັບໂຫຼດໂຟນເດີໃສ່ເວັບເຊີເວີສະແຕຕິກໃດກໍໄດ້ ([packaging/web/README.md](packaging/web/README.md)) |

<br>

<p align="center">
  <a href="https://discord.gg/artcraft"><img alt="ເຂົ້າຮ່ວມ Discord ຂອງຊຸມຊົນ ArtCraft" src="https://img.shields.io/badge/Join%20us%20on%20Discord-5865F2?style=for-the-badge&logo=discord&logoColor=white" height="40"></a>
</p>

<h3 align="center"><a id="come-make-things-with-us"></a>ມາສ້າງສັນສິ່ງຕ່າງໆໄປນຳກັນ</h3>

<p align="center">
  Discord ຂອງພວກເຮົາແມ່ນສູນລວມຜູ້ສ້າງສັນຈາກທຸກຂົງເຂດ. ຕັ້ງແຕ່ນັກແຕ້ມຮູບ, ນັກຖ່າຍຮູບ, ນັກແຕ້ມສະເກັດ, ນັກຕັດຕໍ່ວິດີໂອ,
  ນັກອອກແບບຕົວອັກສອນ ໄປຈົນເຖິງຜູ້ທີ່ກຳລັງຄົ້ນຫາສິ່ງທີ່ຕົນເອງມັກ. ມາແບ່ງປັນຜົນງານທີ່ກຳລັງເຮັດ,
  ຂໍຄວາມຊ່ວຍເຫຼືອ, ແຈ້ງສ່ວນທີ່ມີບັນຫາ, ແລະ ບອກສິ່ງທີ່ທ່ານຢາກໃຫ້ລະບົບເຮັດໄດ້.
  ບໍ່ວ່າທ່ານຈະເຮັດວຽກຫຍັງ ຫຼື ຫາກໍ່ເລີ່ມຕົ້ນມາໄດ້ດົນປານໃດກໍຕາມ, ທີ່ນີ້ຍິນດີຕ້ອນຮັບທຸກຄົນສະເໝີ.
</p>

<p align="center">
  <a href="https://discord.gg/artcraft"><b>discord.gg/artcraft</b></a> ·
  <a href="https://getartcraft.com/">getartcraft.com</a> ·
  <a href="https://getartcraft.com/apps">The Crafting Apps</a> ·
  <a href="https://github.com/storytold/craft-anything">Craft Anything</a>
</p>

---

## <a id="the-craft-app-suite"></a>The Craft App Suite

|                                                                                                                                                                                            | ແອັບ                 | ການນຳໃຊ້                                                     | ໂຄດ                                                  | ເບິ່ງເພີ່ມເຕີມ                                             |
|:------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------:| ------------------ | -------------------------------------------------------- | ----------------------------------------------------- | ------------------------------------------------------- |
| <img src="https://raw.githubusercontent.com/storytold/photocraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.photocraft.png" alt="" width="32" height="32">                     | **PhotoCraft**     | ການແກ້ໄຂຮູບພາບ: ເລເຢີ, ໜ້າກາກ, ຂໍ້ຄວາມ, ໄຟລ໌ PSD ແທ້       | [GitHub](https://github.com/storytold/photocraft)     | [ເວັບໄຊ](https://getartcraft.com/apps/photocraft)     |
| <img src="https://raw.githubusercontent.com/storytold/vectorcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.vectorcraft.png" alt="" width="32" height="32">                   | **VectorCraft**    | ພາບປະກອບແບບເວັກເຕີ                                      | [GitHub](https://github.com/storytold/vectorcraft)    | [ເວັບໄຊ](https://getartcraft.com/apps/vectorcraft)    |
| <img src="https://raw.githubusercontent.com/storytold/filmcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.filmcraft.png" alt="" width="32" height="32">                       | **FilmCraft**      | ການຕັດຕໍ່ວິດີໂອ, ການປັບສີ, ສຽງ                                | [GitHub](https://github.com/storytold/filmcraft)      | [ເວັບໄຊ](https://getartcraft.com/apps/filmcraft)      |
| <img src="https://raw.githubusercontent.com/storytold/lightcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.lightcraft.png" alt="" width="32" height="32">                     | **LightCraft**     | ຄັງຮູບພາບ ແລະ ການລ້າງໄຟລ໌ RAW                                   | [GitHub](https://github.com/storytold/lightcraft)     | [ເວັບໄຊ](https://getartcraft.com/apps/lightcraft)     |
| <img src="https://raw.githubusercontent.com/storytold/pdfcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.pdfcraft.png" alt="" width="32" height="32">                         | **PdfCraft**       | ອ່ານ, ຈັດລະບຽບ, ແລະ ປົກປ້ອງ PDF                                     | [GitHub](https://github.com/storytold/pdfcraft)       | [ເວັບໄຊ](https://getartcraft.com/apps/pdfcraft)       |
| <img src="https://raw.githubusercontent.com/storytold/effectcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.effectcraft.png" alt="" width="32" height="32">                   | **EffectCraft**    | ໂມຊັນກຣາຟິກ ແລະ ເອັບເຟັກສາຍຕາ                                  | [GitHub](https://github.com/storytold/effectcraft)    | [ເວັບໄຊ](https://getartcraft.com/apps/effectcraft)    |
| <img src="https://raw.githubusercontent.com/storytold/designcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.designcraft.png" alt="" width="32" height="32">                   | **DesignCraft**    | ການຈັດວາງໜ້າ ແລະ ວຽກສິ່ງພິມ                                   | [GitHub](https://github.com/storytold/designcraft)    | [ເວັບໄຊ](https://getartcraft.com/apps/designcraft)    |
| <img src="https://raw.githubusercontent.com/storytold/figcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.figcraft.png" alt="" width="32" height="32">                         | **FigCraft**       | ການອອກແບບອິນເຕີເຟສ ແລະ ຕົ້ນແບບ                         | [GitHub](https://github.com/storytold/figcraft)       | [ເວັບໄຊ](https://getartcraft.com/apps/figcraft)       |
| <img src="https://raw.githubusercontent.com/storytold/wordcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.wordcraft.png" alt="" width="32" height="32">                       | **WordCraft**      | ການປະມວນຜົນຄຳ ແລະ ເອກະສານ                                     | [GitHub](https://github.com/storytold/wordcraft)      | [ເວັບໄຊ](https://getartcraft.com/apps/wordcraft)      |
| <img src="https://raw.githubusercontent.com/storytold/soundcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.soundcraft.png" alt="" width="32" height="32">                     | **SoundCraft**     | ການອັດ ແລະ ຕັດຕໍ່ສຽງ                                       | [GitHub](https://github.com/storytold/soundcraft)     | [ເວັບໄຊ](https://getartcraft.com/apps/soundcraft)     |
| <img src="https://raw.githubusercontent.com/storytold/deckcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.deckcraft.png" alt="" width="32" height="32">                       | **DeckCraft**      | ການນຳສະເໜີ ແລະ ສະໄລດ໌                               | [GitHub](https://github.com/storytold/deckcraft)      | [ເວັບໄຊ](https://getartcraft.com/apps/deckcraft)      |
| <img src="https://raw.githubusercontent.com/storytold/cadcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.cadcraft.png" alt="" width="32" height="32">                         | **CADCraft**       | ການອອກແບບດ້ວຍຄອມພິວເຕີ (CAD)                                    | [GitHub](https://github.com/storytold/cadcraft)       | [ເວັບໄຊ](https://getartcraft.com/apps/cadcraft)       |
| <img src="https://raw.githubusercontent.com/storytold/gridcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.gridcraft.png" alt="" width="32" height="32">                       | **GridCraft**      | ສະເປຣດຊີດ ແລະ ຂໍ້ມູນ                                    | [GitHub](https://github.com/storytold/gridcraft)      | [ເວັບໄຊ](https://getartcraft.com/apps/gridcraft)      |
| <img src="https://raw.githubusercontent.com/storytold/texturecraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.texturecraft.png" alt="" width="32" height="32">                 | **TextureCraft**   | ການແຕ້ມລວດລາຍ 3D                                         | [GitHub](https://github.com/storytold/texturecraft)   | [ເວັບໄຊ](https://getartcraft.com/apps/texturecraft)   |
| <img src="https://raw.githubusercontent.com/storytold/materialcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.materialcraft.png" alt="" width="32" height="32">               | **MaterialCraft**  | ວັດສະດຸແບບໂປຣຊີເຈີຣັລອີງຕາມໂນດ                                       | [GitHub](https://github.com/storytold/materialcraft)  | [ເວັບໄຊ](https://getartcraft.com/apps/materialcraft)  |
| <img src="https://raw.githubusercontent.com/storytold/craft-anything/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.craft-anything.png" alt="" width="32" height="32">             | **Craft Anything** | ສຶກສາວິເຄາະຊອບແວທີ່ຕ້ອງການ, ແລ້ວສ້າງຜົນງານຂອງທ່ານເອງ | [GitHub](https://github.com/storytold/craft-anything) | [ເວັບໄຊ](https://getartcraft.com/apps/craft-anything) |
| <img src="https://raw.githubusercontent.com/tangency-labs/solidcraft/main/assets/icons/app/linux/exports/hicolor/64x64/apps/io.tangencylabs.solidcraft.png" alt="" width="32" height="32"> | **SolidCraft**     | 3D CAD ແບບພາຣາມິຕຣິກ ຈາກ Tangency Labs (ແອັບຊຸມຊົນ)           | [GitHub](https://github.com/tangency-labs/solidcraft) |                                                         |

## <a id="license-and-credits"></a>ໃບອະນຸຍາດ ແລະ ເຄຣດິດ

Craft Anything ໃຊ້ໃບອະນຸຍາດແບບຄູ່ [MIT](LICENSE-MIT) ຫຼື [Apache-2.0](LICENSE-APACHE), ໂດຍທ່ານສາມາດເລືອກຝ່າຍໃດກໍໄດ້ຕາມຕ້ອງການ.
ສະຫງວນລິຂະສິດ (c) 2026 ArtCraft Team ແລະ ຜູ້ຮ່ວມປະກອບສ່ວນ Craft Anything ([CONTRIBUTORS.md](CONTRIBUTORS.md)). ແຈ້ງການທີ່ຈຳເປັນແມ່ນມີຢູ່ໃນ [NOTICE](NOTICE).

ຟອນ, ໄອຄອນ, ຮູບພາບ ແລະ ຊັບສິນອື່ນໆທີ່ແນບມານຳ ຍັງຄົງຮັກສາໃບອະນຸຍາດແບບເປີດຂອງແຕ່ລະອັນ. ຜູ້ສ້າງ, ແຫຼ່ງທີ່ມາ ແລະ ໃບອະນຸຍາດຂອງແຕ່ລະລາຍການແມ່ນລະບຸໄວ້ໃນ [ATTRIBUTION.md](ATTRIBUTION.md).

ສິ່ງທີ່ປາກົດໃນຮູບຖ່າຍໜ້າຈໍແມ່ນໂຄງການຕົວຢ່າງທີ່ສ້າງຂຶ້ນເພື່ອຈຸດປະສົງດັ່ງກ່າວ: example.com, ເກມງູ, ໂປຣແກຣມຄິດໄລ່ເລກຂະໜາດນ້ອຍ.

ຊື່ ArtCraft, ເຄື່ອງໝາຍຂໍ້ຄວາມ, ແລະ ໂລໂກ້ ທີ່ຢູ່ໃນ [`docs/brand/`](docs/brand/) ແມ່ນເຄື່ອງໝາຍການຄ້າຂອງ ArtCraft Team ແລະ ບໍ່ໄດ້ຢູ່ພາຍໃຕ້ໃບອະນຸຍາດນີ້. ສາມາດນຳໃຊ້ໄດ້ສະເພາະໃນສະພາບທີ່ບໍ່ມີການດັດແກ້, ເປັນສ່ວນໜຶ່ງຂອງຄັງເກັບໂຄດນີ້ ແລະ Craft Anything ເທົ່ານັ້ນ, ໂດຍຕ້ອງປະຕິບັດຕາມ [`docs/brand/LICENSE-brand.txt`](docs/brand/LICENSE-brand.txt). ໂຄງການທີ່ແຍກອອກໄປ ແລະ ສະບັບດັດແກ້ຕ້ອງລຶບສິ່ງເຫຼົ່ານີ້ອອກ.

<p align="center">
  <a href="https://getartcraft.com/"><img alt="ArtCraft" src="docs/brand/artcraft-mark.svg" width="28"></a><br>
  <sub>ສ້າງຂຶ້ນໂດຍທີມງານ ແລະ ຊຸມຊົນ <a href="https://getartcraft.com/">ArtCraft</a>.</sub>
</p>

---

<sub>ເອກະສານນີ້ແມ່ນສະບັບແປພາສາລາວຂອງ [README.md](README.md). ຖ້າຫາກເນື້ອໃນແຕກຕ່າງກັນ ໃຫ້ຖືເອົາສະບັບພາສາອັງກິດເປັນຫຼັກ.</sub>
