<p align="center">
  <a href="https://getartcraft.com/">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="docs/brand/artcraft-logo-white.svg">
      <img alt="ArtCraft" src="docs/brand/artcraft-logo.svg" width="200">
    </picture>
  </a>
</p>

<p align="center">
  <img src="assets/app-icon/hicolor/256x256/apps/ai.storyteller.craft-anything.png" width="128" height="128" alt="סמל Craft Anything: עורב חרוט על אריח כחול">
</p>

<h1 align="center">Craft Anything</h1>

<p align="center">
  <a href="README.md">English</a> · <a href="README.ja.md">日本語</a> · <a href="README.zh.md">简体中文</a> · <a href="README.zh-hant.md">繁體中文</a> · <a href="README.ko.md">한국어</a> · <a href="README.es.md">Español</a> · <a href="README.fr.md">Français</a> · <a href="README.it.md">Italiano</a> · <a href="README.vi.md">Tiếng Việt</a> · <a href="README.pt.md">Português</a> · <a href="README.th.md">ไทย</a> · <a href="README.tr.md">Türkçe</a> · <b>עברית</b>
</p>

<div dir="rtl">

<p align="center">
  <b>ראו כיצד כל אתר, אפליקציה או משחק עובדים, ותנו ל-AI לבנות גרסה משלכם; מנוע קוד פתוח בשיטת חדר נקי (clean-room) שנכתב ב-Rust טהור.</b><br>
  הצביעו על יעד, ציינו מה תרצו לבנות, והוא יחקור את המקור באמצעות כלי בדיקה אמיתיים,<br>
  יפעיל סוכן קידוד לבניית פרויקט חדש, ישווה את התוצאה למקור וילטש אותה.
</p>

<p align="center">
  <img alt="100% Rust" src="https://img.shields.io/badge/100%25-Rust-b7410e?style=flat-square&logo=rust">
  <img alt="macOS · Windows · Linux · FreeBSD · Web" src="https://img.shields.io/badge/macOS%20%C2%B7%20Windows%20%C2%B7%20Linux%20%C2%B7%20FreeBSD%20%C2%B7%20Web-native-2d81ff?style=flat-square">
  <img alt="License: MIT OR Apache-2.0" src="https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-3a3a3a?style=flat-square">
  <img alt="Status: early alpha" src="https://img.shields.io/badge/status-early%20alpha-d69e2e?style=flat-square">
  <br>
  <a href="https://sonarcloud.io/project/overview?id=storytold_craft-anything"><img alt="Scanned by SonarCloud" src="https://img.shields.io/badge/Scanned_by-SonarCloud-4b9fd5?style=flat-square&logo=sonarqubecloud&logoColor=white"></a>
  <a href="https://coderabbit.ai"><img alt="Reviewed by CodeRabbit" src="https://img.shields.io/coderabbit/prs/github/storytold/craft-anything?style=flat-square&label=Reviewed%20by%20CodeRabbit&logo=coderabbit"></a>
</p>

<p align="center">
  <a href="https://discord.gg/artcraft"><img alt="הצטרפו לקהילת ArtCraft ב-Discord" src="https://img.shields.io/badge/Join%20us%20on%20Discord-5865F2?style=for-the-badge&logo=discord&logoColor=white" height="40"></a>
</p>

<p align="center">
  <a href="#downloads"><b>הורדת Craft Anything</b></a> ·
  <a href="https://getartcraft.com/">ArtCraft</a> ·
  <a href="https://getartcraft.com/apps">כל אפליקציות Crafting</a>
</p>

<br>

<p align="center">
  <img src="docs/images/project-website.png" alt="פרויקט אתר ב-Craft Anything: הבדיקה מפרטת מה טרם הותאם וכמה כל עמוד דומה למקור, הבונה מודיע שהגרסה מוכנה, כרטיס אישור מבקש אישור לפני הרצת פקודה, והתוכנית מציגה שני שלבים שהושלמו מתוך ארבעה" width="100%">
  <br>
  <sub>פרויקט אתר: כל בדיקה מראה עד כמה הבנייה דומה למקור, ניתן לאשר או לדחות כל פעולה החורגת מהכללים, והתוכנית מציגה מה נשאר.</sub>
</p>

> [!NOTE]
> **ArtCraft היא קהילה של יוצרים ואמנים מכל התחומים.** ציירים, צלמים,
> קולנוענים, מאיירים, מעצבים, אנימטורים, חובבים וגם מי שהרימו עיפרון
> רק בשבוע שעבר. אם אתם יוצרים דברים — מקומכם איתנו. **[בואו לומר שלום ב-Discord.](https://discord.gg/artcraft)**

<p align="center">
  <a href="#how-it-works">איך זה עובד</a> ·
  <a href="#two-ways-to-use-it">שתי דרכים לעבוד</a> ·
  <a href="#ai">AI</a> ·
  <a href="#built-for-agents">סוכנים</a> ·
  <a href="#under-the-hood">מתחת למכסה</a> ·
  <a href="#get-started">התחלה מהירה</a> ·
  <a href="#status">סטטוס</a> ·
  <a href="#downloads">הורדות</a> ·
  <a href="https://discord.gg/artcraft">Discord</a>
</p>

<br>

<table>
  <tr>
    <td width="25%" valign="top">
      <h3>🔍 לחקור כל דבר</h3>
      אתרים, אפליקציות שולחן עבודה, אפליקציות טלפון, משחקים וקבצים: כלי בדיקה אמיתיים מגלים כיצד הם בנויים ומה הם עושים, עם הראיות שמאחורי כל תשובה.
    </td>
    <td width="25%" valign="top">
      <h3>🛠️ לבנות גרסה משלכם</h3>
      ציינו מה תרצו (״לשחזר את האתר הזה״, ״לשחזר את המשחקיות של המשחק הזה ב-Rust״), וסוכן קידוד יבנה פרויקט חדש בטכנולוגיה שתבקשו.
    </td>
    <td width="25%" valign="top">
      <h3>✅ נבדק, לא מנוחש</h3>
      כל בנייה מושווית למקור (עמודים בשלושה רוחבי מסך, בדיקה עצמית למשחק או ריצות של אפליקציה) ומשופרת עד להתאמה קרובה.
    </td>
    <td width="25%" valign="top">
      <h3>🤖 מוכן לסוכנים</h3>
      320 כלים, כל אחד מהם הוא גם פקודת CLI וגם כלי MCP, כך שכל סוכן יכול להפעיל את אותו מנוע של האפליקציה. אין צורך ב-AI כדי לבצע בדיקה.
    </td>
  </tr>
</table>

<br>

<a id="how-it-works"></a>

## איך זה עובד

הצביעו על יעד ב-Craft Anything, ציינו מה תרצו לבנות ולחצו על **Start**. הכלי עובד בארבעה שלבים, ושואל אתכם בשפה פשוטה כשנדרשת הכרעה שלכם:

1. **חקירה (Study):** בדיקת המקור — העמודים שלו בכל רוחב מסך, הקוד, הקבצים ואופן הריצה שלו.
2. **בנייה (Build):** סוכן קידוד בונה פרויקט חדש בשפה, בפריימוורק או במנוע שציינתם בבקשה (או בברירת מחדל הגיונית).
3. **בדיקה (Check):** השוואת הבנייה למקור והצגת רשימה של מה שעדיין לא תואם.
4. **מוכן (Ready):** פתיחה, משחק או תצוגה מקדימה של התוצאה, או בקשת שינויים לליטוש נוסף.

<table>
  <tr>
    <td width="50%" valign="top">
      <img src="docs/images/make.png" alt="מסך הבנייה: בחירת אתר, אפליקציה או משחק לשחזור, הזנת הכתובת, בחירה בין Clean room ל-Deep, תיאור המטרה ולחיצה על Start" width="100%"><br>
      <sub><b>בנייה:</b> בוחרים מה לשחזר, מתארים מה רוצים ולוחצים Start.</sub>
    </td>
    <td width="50%" valign="top">
      <img src="docs/images/project-game.png" alt="פרויקט משחק סנייק מוכן עם כל השלבים שהושלמו וכפתור להפעלת המשחק" width="100%"><br>
      <sub><b>מוכן:</b> מפעילים את המשחק, או מבקשים שינויים למטה.</sub>
    </td>
  </tr>
</table>

<a id="two-ways-to-use-it"></a>

## שתי דרכים לעבוד

- **פשוט (Simple, ברירת מחדל):** ״מה תרצו ליצור?״ בוחרים אתר, אפליקציה או משחק, מתארים מה רוצים ולוחצים Start. עמוד הפרויקט מציג את ארבעת השלבים (חקירה ← בנייה ← בדיקה ← מוכן), מה הסוכן עושה במילים פשוטות, כרטיסי שאלות ואישורים, עד כמה התוצאה דומה למקור, וכפתורים לפתיחת התוצר או התיקייה שלו. ניתן לבקש שינויים בתיבה שבתחתית המסך. אפשרות ״להסתכל מבפנים בלבד״ פותחת יעד לצורך שאלות בלי לבנות דבר (במצב Deep).
- **מתקדם (Advanced):** סביבת העבודה המלאה — כל כלי (Ctrl/Cmd+K), פונקציות, מחרוזות, פסאודו-C, אסמבלי (Disassembly), Hex, גרפי קריאות, ראיות, נעלמים, עבודות, יומני לוג ומסוף הסוכן.

שני המצבים מפעילים את אותו המנוע.

## חדר נקי (Clean room) או עמוק (Deep)

- **Clean room (ברירת מחדל):** דבר מתוך המקור אינו נקרא — לא הקוד שלו, לא הקבצים ולא קוד המקור של העמוד. Craft Anything עובד על סמך מה שניתן לראות: תמונות שהוספתם, האתר כפי שמבקר רואה אותו (צילומי מסך, טקסט גלוי, פריסה מדודה, צבעים, גופנים, מרווחים ותפריטים פתוחים), וכאשר תאשרו זאת — תמונה של חלון האפליקציה. כלים שמבקשים להציץ פנימה נדחים עם הסבר כיצד להפעיל את מצב Deep.
- **Deep:** כל הכלים משמשים אתכם, לשימושכם הפרטי. קוד התוכנה וקבציה נקראים באמצעות המנועים המובנים (ועם Ghidra כשהוא מותקן), ה-JavaScript של אתר נקרא ועמוד הפתיחה שלו מנוטר, והבונה רשאי להשתמש בכל הממצאים.

חדר נקי הוא כלל לגבי מה ש-Craft Anything מציע ומה הוא דוחה. הוא אינו ארגז חול (sandbox) ברמת מערכת ההפעלה ואינו מבודד את המחשב. סוכן קידוד שאתם מריצים עדיין פועל תחת ההרשאות שלכם, ומה שהוא יכול לקרוא, לכתוב או להריץ תלוי בהרשאותיו. היכולת של Craft Anything לחסום כלים של הסוכן עצמו משתנה מסוכן לסוכן; לגבי הכלים של Claude Code בשיחות ובשאלות, ראו [`docs/claude-code.md`](docs/claude-code.md).

ניתן להחליף מצב בשורת הכותרת או בהגדרות, באמצעות `settings_set deep_mode true`, או להרצת CLI חד-פעמית עם `--deep` (או `--clean-room`).

<p align="center">
  <img src="docs/images/workbench.png" alt="סביבת העבודה המתקדמת: רשימת פונקציות, פסאודו-C של הפונקציה הנבחרת, הסוכן עונה על שאלה באמצעות קריאת כלי, ויומן הראיות" width="100%">
  <br>
  <sub><b>מתקדם:</b> פונקציות, פסאודו-C, הסוכן והראיות שמאחורי כל תשובה.</sub>
</p>

<a id="ai"></a>

## AI

הבנייה דורשת סוכן קידוד במחשבכם. **Claude Code** נתמך ראשון: Craft Anything מריץ את תוכנת `claude` המקורית והלא-משונה שלכם, שמשתמשת בהתחברות שלכם; Craft Anything לעולם אינו רואה את החשבון שלכם ואינו מטפל בו. השימוש שלכם בו כפוף להסכם שלכם עם Anthropic. ראו [docs/claude-code.md](docs/claude-code.md) להסבר כיצד ומדוע. מתאם ל-Codex קיים אך טרם אומת בהתקנה אמיתית. **GitHub Copilot** יכול להשתמש ב-Copilot CLI המותקן אצלכם דרך ACP: יש להתחבר בנפרד עם `copilot login`. **Grok Build** יכול להשתמש בתוכנת `grok` המותקנת דרך ACP (`grok agent stdio`): יש להתחבר בנפרד עם `grok login`. ראו [docs/grok.md](docs/grok.md). **Kimi Code CLI** יכול להשתמש בתוכנת `kimi` המותקנת דרך ACP (`kimi acp`): יש להתחבר בנפרד עם `kimi login`; מתאם זה טרם הורץ בשידור חי. סוכנים נוספים שמדברים ACP (Gemini CLI, Qwen Code, opencode, goose, Hermes, Maki, Devin) ניתנים לבחירה בכל מנוע קצה; מביניהם Maki ו-Devin הורצו בשידור חי (Devin מזדהה באמצעות המפתח השמור של מנוע הקצה — ההתחברות ל-CLI שלו אינה תקפה דרך ACP). Anthropic, OpenAI, OpenRouter, xAI (Grok, כשמוגדר `XAI_API_KEY`) ושרתים מקומיים תואמי OpenAI זמינים לצורך שאלות וניתוח (הגדרות ← AI).

<a id="built-for-agents"></a>

## בנוי לסוכנים

כל כלי הוא גם תת-פקודת CLI וגם כלי MCP, כך שסוכן יכול לחקור יעד או להריץ בנייה מחדש שלמה בלי האפליקציה הגרפית:

```sh
craft-anything-cli commands                                   # כל הכלים, לפי ערכת כלים
craft-anything-cli mcp                                        # הגשת הכלים לכל לקוח MCP
craft-anything-cli rebuild https://example.com "Recreate this website"
```

התוצאות הן רשומות ראיות הכוללות רמת ביטחון, כיסוי ומגבלות; תוצאות גדולות חוזרות תחילה כתקציר וניתנות לדפדוף באמצעות `core_evidence_browse`. הפקודה `craft-anything-cli setup` מחברת את Craft Anything לאפליקציות של סוכני קידוד.

<a id="under-the-hood"></a>

## מתחת למכסה

מנוע הנדסה לאחור ב-Rust טהור עם 320 כלים (300 במנוע והשאר משכבות ה-AI וההגדרה) לתוכנות נייטיב, .NET, Java, Android, Apple, Electron/JavaScript, אתרים, קושחה, לכידות מנות (packet captures), מסדי נתונים וקובצי משחקים. המפענחים, הדיסאסמבלר והדה-קומפילר שלו פועלים ללא כל תוכנה חיצונית; מצב Deep משתמש בכולם, וגם ב-Ghidra כשהוא מותקן. המנוע מניע את אפליקציית שולחן העבודה (`egui`), את ה-CLI ואת שרת ה-MCP, וחלקים ממנו רצים בדפדפן כ-WebAssembly. הרשימה המלאה נמצאת ב-[docs/tools.md](docs/tools.md).

<a id="get-started"></a>

## התחלה מהירה

דורש Rust בגרסה **1.97 ומעלה** (תלות מסוימת דורשת זאת; `rustup update`). ב-Debian או Ubuntu, אפליקציית שולחן העבודה דורשת גם:

```sh
sudo apt-get install build-essential libxkbcommon-dev libxkbcommon-x11-0 libwayland-dev libx11-dev libxrandr-dev libxi-dev libgl1-mesa-dev libgtk-3-dev
```

לאחר מכן:

```sh
git clone https://github.com/storytold/craft-anything && cd craft-anything
cargo run --release -p craft-anything                # אפליקציית שולחן העבודה
cargo install --path apps/craft-anything-cli         # כלי שורת הפקודה, כ-`craft-anything-cli`
craft-anything-cli doctor                            # בדיקת ההתקנה, סוכני הקידוד והמנועים
```

הבנייה דורשת סוכן קידוד מחובר במחשבכם (ראו [AI](#ai)); פקודת `doctor` תציין מה חסר. הרצה מחדש משורת הפקודה (שאלות מוצגות במסוף):

```sh
craft-anything-cli rebuild https://example.com "Recreate this website"
craft-anything-cli rebuild - "A snake game in Rust" --rounds 1
craft-anything-cli rebuild https://example.com "Make the header blue" --continue-in "<project folder>"
craft-anything-cli rebuild - "Add a contact form" --project-folder "<your folder>" --existing edit
```

פרויקטים חדשים נשמרים ב-`Documents/Craft Anything` (ניתן לשינוי בהגדרות ← כללי). כל תיקיית פרויקט מכילה `craft.json` (הבקשה, היעד, הסטטוס וההיסטוריה), `capture/` (מה שהוקלט מהיעד), `app/` (הפרויקט שנוצר), `checks/` (כל השוואה למקור), `investigation.cany/` (ראיות) ו-`inbox/` (שאלות פתוחות).

פרויקט יכול להתחיל גם בתיקייה שתבחרו (Choose a folder… תחת Start, או `--project-folder`). אם כבר יש בה קבצים, יש לציין מה הם: האפליקציה עצמה, שאותה הבונה עורך במקום, או חומר גלם להתחיל ממנו, כשהאפליקציה נבנית ב-`app/` לצד הקבצים (Settings → General יכול לזכור את התשובה). Settings → General מציג גם פרויקטים מתיקיות נוספות; הסרה מהרשימה רק מוציאה את הפרויקט מהרשימה.

<a id="status"></a>

## סטטוס

> [!IMPORTANT]
> **סטטוס:** Craft Anything נמצא באלפא מוקדמת. מה עובד היום, מה חלקי ומה הלאה:
>
> - **עובד היום:** אפליקציית שולחן העבודה (פשוט ומתקדם), ה-CLI ושרת ה-MCP; תהליך חקירה ← בנייה ← בדיקה ← מוכן עבור אתרים, אפליקציות ומשחקים עם Claude Code כבונה; בדיקות אתרים בשלושה רוחבי מסך; שחזורי משחקים ב-Unity, Unreal או Godot; מצב Deep עם Ghidra; תמונות ומודלי תלת-ממד מיוצרים (אופציונלי, עם מפתח fal.ai אישי); תוספים (שרתי MCP, ורכיבי WASM בבניות עם האפשרות `wasm-plugins`).
> - **חלקי:** בנייה עם מפתחות API וסוכני קידוד אחרים (מוכן בקוד, טרם הורץ בשידור חי), בדיקות התנהגות לאפליקציות ומשחקים, וכמה כלי מנוע שנבדקו רק מול דגימות מדומות.
> - **פורסם:** גרסאות פומביות עם מתקינים ל-Windows, macOS, Linux ו-FreeBSD מפורסמות ב-[דף הגרסאות (Releases)](https://github.com/storytold/craft-anything/releases); הן עדיין בגרסת אלפא.
> - **הבא בתור:** השכבה המנוהלת.
>
> ב-[docs/STATUS.md](docs/STATUS.md) נמצאים הפירוט וכיצד כל חלק נבדק,
> ב-[docs/tool-status.md](docs/tool-status.md) נמצאת הרשימה כלי-אחר-כלי,
> וב-[docs/ROADMAP.md](docs/ROADMAP.md) נמצאת התוכנית, סעיף אחר סעיף. אנא דווחו על תקלות (issues), או ספרו לנו מה נשבר ב-[Discord](https://discord.gg/artcraft).

**שימוש אחראי.** השתמשו ב-Craft Anything בתוכנה שבבעלותכם או שמותר לכם לבחון. הכלי אינו עוקף DRM, מסכי התחברות או מנגנוני אנטי-צ׳יט. שחזורים משחזרים פריסה, סגנון והתנהגות מתוך ראיות; הסוכן מונחה שלא להעתיק סמלילים, תמונות או טקסטים אלא אם אישרתם שהם בבעלותכם או שיש לכם רשות לכך.

**התקנת תלויות מריצה קוד.** כאשר בונה מתקין את התלויות של פרויקט, הפקודות רצות על המחשב שלכם, והחבילות עלולות להריץ קוד בזמן ההתקנה. npm מריץ כברירת מחדל את סקריפטי ההתקנה של כל חבילה, וסקריפטי בנייה של Rust וחבילות מקור של Python מריצים קוד הגדרה משלהם. Craft Anything שואל לפני פקודה מסוג שעדיין לא נתקל בו בפרויקט זה (`npm`, `cargo`…). בחירה ב-״always allow״ עבור `npm` תאפשר לכל פקודות npm הבאות לרוץ בלי לשאול, לכן עשו זאת רק בפרויקט שאתם סומכים על התלויות שלו. כדי להתקין חבילות npm בלי סקריפטי ההתקנה שלהן, הריצו `npm install --ignore-scripts`.

<a id="downloads"></a>

## הורדות

ב-GitHub, [הגרסה האחרונה](https://github.com/storytold/craft-anything/releases/latest) כוללת את כל הבניות המפורטות מטה, וב-[כל הגרסאות](https://github.com/storytold/craft-anything/releases) נמצאות גרסאות קודמות והערות השחרור שלהן. `<ver>` בשמות הקבצים הוא מספר הגרסה, ו-`SHA256SUMS.txt` מפרט סכום ביקורת (checksum) לכל קובץ.

### Windows

| בנייה | מתקין | ניידת (Portable) |
|---|---|---|
| x64 (Intel/AMD 64-bit) | `craft-anything-<ver>-windows-x64.msi` | `craft-anything-<ver>-windows-x64-portable.zip` |
| arm64 (Snapdragon ומחשבי ARM אחרים) | `craft-anything-<ver>-windows-arm64.msi` | `craft-anything-<ver>-windows-arm64-portable.zip` |

המתקינים וקובצי ההפעלה חתומים דיגיטלית. קובץ ה-zip הנייד שומר את ההגדרות שלו לצד האפליקציה.

### macOS

| בנייה | קובץ | הערות |
|---|---|---|
| אפליקציה, אוניברסלית (Apple silicon + Intel) | `craft-anything-<ver>-macos-universal.dmg` | חתומה ומאושרת נוטריזציה |
| כלי שורת פקודה, אוניברסלי | `craft-anything-cli-<ver>-macos-universal.zip` | חתום ומאושר נוטריזציה; ראו מטה אם macOS חוסם אותו |

גרסאות שאחרי 0.2.0 כוללות את כלי שורת הפקודה בתוך האפליקציה, כך שבניות חדשות אינן זקוקות לקובץ ה-zip הנפרד. תצטרכו אותו רק כדי להשתמש בשורת הפקודה בלי האפליקציה.

הורדה של קובץ ה-zip דרך הדפדפן עלולה להיעצר עם ההודעה "Apple could not verify craft-anything-cli" (תוכנה בודדת אינה יכולה לשאת איתה את כרטיס הנוטריזציה, ולכן macOS נדרשת לבדוק אותו מקוונת). הורדה מהמסוף אינה נבדקת בדרך זו, ולכן השתמשו בפקודות הבאות:

```sh
ver=0.2.0   # החליפו בגרסה הרצויה
curl -fLO "https://github.com/storytold/craft-anything/releases/download/v$ver/craft-anything-cli-$ver-macos-universal.zip"
ditto -x -k "craft-anything-cli-$ver-macos-universal.zip" .
mkdir -p ~/.local/bin
mv "craft-anything-cli-$ver-macos-universal/craft-anything-cli" ~/.local/bin/
~/.local/bin/craft-anything-cli --version
```

כדי להריץ אותו פשוט כ-`craft-anything-cli`, הוסיפו את `~/.local/bin` למשתנה `PATH`.

אם העותק החסום כבר נמצא אצלכם, לחצו Done (ולא Move to Bin), ואז הריצו `xattr -d com.apple.quarantine <path to craft-anything-cli>`, או אשרו אותו תחת System Settings → Privacy & Security → Open Anyway.

#### בדיקה שהורדה ל-Mac חתומה ומאושרת נוטריזציה

כל גרסה ל-Mac חתומה באישור Developer ID ומאושרת נוטריזציה על ידי Apple. כדי לבדוק הורדה בעצמכם, הריצו את הפקודות הבאות במסוף (החליפו את `<ver>` בגרסה ואת הנתיבים בנתיבים שלכם):

```sh
# תמונת הדיסק: "accepted" ו-"source=Notarized Developer ID" משמעותם שהבדיקה עברה
spctl --assess --type open --context context:primary-signature -v "craft-anything-<ver>-macos-universal.dmg"
# כרטיס הנוטריזציה מצורף: מדפיס "The validate action worked!"
xcrun stapler validate "craft-anything-<ver>-macos-universal.dmg"

# האפליקציה, לאחר פתיחת תמונת הדיסק: החתימה חייבת להתאמת
codesign --verify --deep --strict --verbose=2 "/Volumes/<disk image name>/Craft Anything.app"
spctl --assess --type execute --verbose "/Volumes/<disk image name>/Craft Anything.app"

# כלי שורת הפקודה: תוכנה בודדת אינה יכולה לשאת כרטיס, ולכן macOS בודקת אותה מקוונת
codesign --verify --strict --verbose=2 ~/.local/bin/craft-anything-cli
codesign -dv --verbose=4 ~/.local/bin/craft-anything-cli 2>&1 | grep Authority
```

שורות ה-`Authority` צריכות לציין אישור מסוג "Developer ID Application". אם אחת הבדיקות נכשלת, אל תריצו את הקובץ; דווחו על כך בצירוף הפלט ותג הגרסה.

### Linux

| פורמט | x86_64 | aarch64 (ARM64) | הערות |
|---|---|---|---|
| AppImage | `craft-anything-<ver>-linux-x86_64.AppImage` | `craft-anything-<ver>-linux-aarch64.AppImage` | רץ מכל מקום; מתעדכן עצמאית עם [AppImageUpdate](https://github.com/AppImageCommunity/AppImageUpdate) (קובצי `.zsync`) |
| Flatpak | `craft-anything-<ver>-linux-x86_64.flatpak` | `craft-anything-<ver>-linux-aarch64.flatpak` | רץ בארגז חול, ולכן אינו יכול להפעיל דפדפן במחשבכם, דבר שנדרש לבדיקת אתרים (השתמשו בחבילה אחרת לכך); `flatpak install --user <file>` |
| Debian/Ubuntu | `craft-anything-<ver>-linux-x86_64.deb` | `craft-anything-<ver>-linux-aarch64.deb` | |
| Fedora/RHEL/openSUSE | `craft-anything-<ver>-linux-x86_64.rpm` | `craft-anything-<ver>-linux-aarch64.rpm` | |
| Tarball | `craft-anything-<ver>-linux-x86_64.tar.gz` | `craft-anything-<ver>-linux-aarch64.tar.gz` | פריסה לכל תיקייה |

בניות RISC-V (`riscv64`) זמינות כ-`craft-anything-<ver>-linux-riscv64.tar.gz`, `craft-anything-<ver>-linux-riscv64.deb`, ו-`craft-anything-<ver>-linux-riscv64.rpm`.

קובץ ה-AppImage מכיל גם את ה-CLI: הריצו `<AppImage> --cli <command>` (לדוגמה `<AppImage> --cli mcp` עבור לקוח MCP), או צרו קישור אל ה-AppImage בשם `craft-anything-cli` והריצו אותו.

### FreeBSD

| בנייה | קובץ |
|---|---|
| x86_64 | `craft-anything-<ver>-freebsd-x86_64.tar.gz` |
| aarch64 (ARM64) | `craft-anything-<ver>-freebsd-aarch64.tar.gz` |

### Web (WebAssembly)

| בנייה | קובץ | הערות |
|---|---|---|
| מציג דפדפן | `craft-anything-web-<ver>.zip` | מראה מהו קובץ, ישירות בדפדפן (הקובץ לעולם אינו יוצא ממנו); ארחו את התיקייה בכל שרת אינטרנט סטטי ([packaging/web/README.md](packaging/web/README.md)) |

<br>

<p align="center">
  <a href="https://discord.gg/artcraft"><img alt="הצטרפו לקהילת ArtCraft ב-Discord" src="https://img.shields.io/badge/Join%20us%20on%20Discord-5865F2?style=for-the-badge&logo=discord&logoColor=white" height="40"></a>
</p>

<h3 align="center">בואו ליצור איתנו דברים</h3>

<p align="center">
  שרת ה-Discord שלנו הוא מקום המפגש של אמנים ויוצרים מכל סוג: אנשים שמציירים, מצלמים, משרטטים, עורכים סרטים,
  מסדרים טיפוגרפיה, וגם כאלה שעדיין מגלים מה הם אוהבים ליצור. שתפו במה שאתם עובדים עליו,
  בקשו עזרה, ספרו לנו מה נשבר, או אילו יכולות הייתם רוצים שהכלים האלה יקבלו.
  יהא התחום שלכם אשר יהא ויהא הוותק שלכם אשר יהא — ברוכים הבאים.
</p>

<p align="center">
  <a href="https://discord.gg/artcraft"><b>discord.gg/artcraft</b></a> ·
  <a href="https://getartcraft.com/">getartcraft.com</a> ·
  <a href="https://getartcraft.com/apps">אפליקציות Crafting</a> ·
  <a href="https://github.com/storytold/craft-anything">Craft Anything</a>
</p>

---

## חבילת אפליקציות Craft

| | אפליקציה | למה היא מיועדת | קוד | עוד מידע |
|:-:|---|---|---|---|
| <img src="https://raw.githubusercontent.com/storytold/photocraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.photocraft.png" alt="" width="32" height="32"> | **PhotoCraft** | עריכת תמונות: שכבות, מסכות, טיפוגרפיה וקובצי PSD אמיתיים | [GitHub](https://github.com/storytold/photocraft) | [אתר](https://getartcraft.com/apps/photocraft) |
| <img src="https://raw.githubusercontent.com/storytold/vectorcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.vectorcraft.png" alt="" width="32" height="32"> | **VectorCraft** | איור וקטורי | [GitHub](https://github.com/storytold/vectorcraft) | [אתר](https://getartcraft.com/apps/vectorcraft) |
| <img src="https://raw.githubusercontent.com/storytold/filmcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.filmcraft.png" alt="" width="32" height="32"> | **FilmCraft** | עריכת וידאו, צבע וסאונד | [GitHub](https://github.com/storytold/filmcraft) | [אתר](https://getartcraft.com/apps/filmcraft) |
| <img src="https://raw.githubusercontent.com/storytold/lightcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.lightcraft.png" alt="" width="32" height="32"> | **LightCraft** | ספריית תמונות ופיתוח RAW | [GitHub](https://github.com/storytold/lightcraft) | [אתר](https://getartcraft.com/apps/lightcraft) |
| <img src="https://raw.githubusercontent.com/storytold/pdfcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.pdfcraft.png" alt="" width="32" height="32"> | **PdfCraft** | קריאה, ארגון והגנה על קובצי PDF | [GitHub](https://github.com/storytold/pdfcraft) | [אתר](https://getartcraft.com/apps/pdfcraft) |
| <img src="https://raw.githubusercontent.com/storytold/effectcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.effectcraft.png" alt="" width="32" height="32"> | **EffectCraft** | גרפיקה בתנועה ואפקטים חזותיים | [GitHub](https://github.com/storytold/effectcraft) | [אתר](https://getartcraft.com/apps/effectcraft) |
| <img src="https://raw.githubusercontent.com/storytold/designcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.designcraft.png" alt="" width="32" height="32"> | **DesignCraft** | עימוד והוצאה לאור | [GitHub](https://github.com/storytold/designcraft) | [אתר](https://getartcraft.com/apps/designcraft) |
| <img src="https://raw.githubusercontent.com/storytold/figcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.figcraft.png" alt="" width="32" height="32"> | **FigCraft** | עיצוב ממשקים ואבות-טיפוס | [GitHub](https://github.com/storytold/figcraft) | [אתר](https://getartcraft.com/apps/figcraft) |
| <img src="https://raw.githubusercontent.com/storytold/wordcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.wordcraft.png" alt="" width="32" height="32"> | **WordCraft** | עיבוד תמלילים ומסמכים | [GitHub](https://github.com/storytold/wordcraft) | [אתר](https://getartcraft.com/apps/wordcraft) |
| <img src="https://raw.githubusercontent.com/storytold/soundcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.soundcraft.png" alt="" width="32" height="32"> | **SoundCraft** | הקלטה ועריכת אודיו | [GitHub](https://github.com/storytold/soundcraft) | [אתר](https://getartcraft.com/apps/soundcraft) |
| <img src="https://raw.githubusercontent.com/storytold/deckcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.deckcraft.png" alt="" width="32" height="32"> | **DeckCraft** | מצגות ושקופיות | [GitHub](https://github.com/storytold/deckcraft) | [אתר](https://getartcraft.com/apps/deckcraft) |
| <img src="https://raw.githubusercontent.com/storytold/cadcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.cadcraft.png" alt="" width="32" height="32"> | **CADCraft** | תיב״ם (CAD) | [GitHub](https://github.com/storytold/cadcraft) | [אתר](https://getartcraft.com/apps/cadcraft) |
| <img src="https://raw.githubusercontent.com/storytold/gridcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.gridcraft.png" alt="" width="32" height="32"> | **GridCraft** | גיליונות אלקטרוניים ונתונים | [GitHub](https://github.com/storytold/gridcraft) | [אתר](https://getartcraft.com/apps/gridcraft) |
| <img src="https://raw.githubusercontent.com/storytold/texturecraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.texturecraft.png" alt="" width="32" height="32"> | **TextureCraft** | צביעת טקסטורות בתלת-ממד | [GitHub](https://github.com/storytold/texturecraft) | [אתר](https://getartcraft.com/apps/texturecraft) |
| <img src="https://raw.githubusercontent.com/storytold/materialcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.materialcraft.png" alt="" width="32" height="32"> | **MaterialCraft** | חומרים פרוצדורליים מבוססי צמתים | [GitHub](https://github.com/storytold/materialcraft) | [אתר](https://getartcraft.com/apps/materialcraft) |
| <img src="https://raw.githubusercontent.com/storytold/craft-anything/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.craft-anything.png" alt="" width="32" height="32"> | **Craft Anything** | לראות איך כל תוכנה עובדת ולבנות אחת משלכם | [GitHub](https://github.com/storytold/craft-anything) | [אתר](https://getartcraft.com/apps/craft-anything) |
| <img src="https://raw.githubusercontent.com/tangency-labs/solidcraft/main/assets/icons/app/linux/exports/hicolor/64x64/apps/io.tangencylabs.solidcraft.png" alt="" width="32" height="32"> | **SolidCraft** | תיב״ם תלת-ממד פרמטרי מבית Tangency Labs (אפליקציה קהילתית) | [GitHub](https://github.com/tangency-labs/solidcraft) | |

## רישיון וקרדיטים

Craft Anything מופץ תחת רישיון כפול [MIT](LICENSE-MIT) או [Apache-2.0](LICENSE-APACHE), לפי בחירתכם.
Copyright (c) 2026 ArtCraft Team ותורמי Craft Anything ([CONTRIBUTORS.md](CONTRIBUTORS.md)). הודעות נדרשות נמצאות ב-[NOTICE](NOTICE).

גופנים, סמלים, תמונות ונכסים אחרים הכלולים בחבילה שומרים על רישיונות הקוד הפתוח שלהם; כל אחד מהם מפורט עם יוצרו, מקורו ורישיונו ב-[ATTRIBUTION.md](ATTRIBUTION.md).

צילומי המסך מציגים פרויקטים לדוגמה שנבנו עבורם: example.com, משחק סנייק ותוכנית מחשבון קטנה.

השם ArtCraft, הסימן המסחרי והלוגואים בתיקייה [`docs/brand/`](docs/brand/) הם סימנים מסחריים של צוות ArtCraft ואינם מכוסים ברישיון זה. ניתן להשתמש בהם רק ללא שינוי, ורק כחלק ממאגר זה ומ-Craft Anything, בכפוף ל-[`docs/brand/LICENSE-brand.txt`](docs/brand/LICENSE-brand.txt). פיצולים (forks) וגרסאות ששונו חייבים להסיר אותם.

</div>

<p align="center">
  <a href="https://getartcraft.com/"><img alt="ArtCraft" src="docs/brand/artcraft-mark.svg" width="28"></a><br>
  <sub>נוצר על ידי צוות <a href="https://getartcraft.com/">ArtCraft</a> והקהילה.</sub>
</p>
