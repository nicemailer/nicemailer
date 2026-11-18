# Localisation : plan de travail

Statut : dates, pluriels et phrases construites par morceaux corrigés dans l'app (`docs/localization.md`) ; erreurs des requêtes, des événements et des résultats décrites par une raison, écrites par l'app, transmises aux agents (section 3.1). Le reste est à faire, dans l'ordre de la section 3.

Ce document est le plan pour qu'une troisième langue ne soit plus qu'une question de traduction. Les règles déjà en place sont dans `docs/localization.md` ; quand une étape est livrée, ses règles y passent et l'étape est rayée ici.

## 1. Constat

Fait :

- **Deux locales** (`Theme.swift`) : `appLocale`, la langue de l'app seule, pour le texte écrit dans un mail ; `uiLocale`, la langue de l'app pour les mots et les réglages du Mac pour l'ordre des dates et l'horloge 12 h / 24 h, pour tout ce que l'interface affiche.
- **Dates par gabarit** (`jmm`, `dMMM`…) : plus aucun format écrit en dur dans l'interface, plus de `fr_FR` imposé au thème classique. La colonne de date de 8 caractères se replie sur des chiffres quand les noms de mois d'une langue sont trop longs.
- **Pluriels** dans le String Catalog, en anglais comme en français : 23 clés à un nombre, 3 à deux nombres (substitutions). `L()` formate avec la locale, sans quoi le français suivrait la règle anglaise (« 0 mails »).
- **Une phrase, une clé** : les phrases construites par concaténation (« label + (and 2 others) », « destinataires + +3 », « nom + (image shown in the body) ») sont devenues des clés avec arguments.
- Le thème classique et les erreurs de connexion au noyau, écrits en dur en français, passent par le catalogue.
- Tests : `LocalizationTests` (arguments de chaque forme, pluriels, clés mortes, texte en dur dans les vues, dates en dix locales) et deux scénarios UI (`localization`, `localization-fr`), qui fixent la langue et la région d'un scénario.

Reste (section 3) : le texte produit par le noyau, le texte écrit dans les mails, la détection des citations, la saisie des dates et des durées, les raccourcis clavier, les nombres, quelques textes hors catalogue, la longueur des traductions et l'écriture de droite à gauche, et la procédure d'ajout d'une langue.

## 2. Principes

1. **Le noyau ne connaît pas la langue de l'utilisateur.** Il décrit ce qui se passe par des codes et des valeurs ; chaque client écrit la phrase. Le CLI l'écrit en anglais (`CLAUDE.md`, « Languages »), l'app dans la langue de l'utilisateur.
2. **Le texte d'un tiers reste tel quel.** La réponse d'un serveur IMAP ou SMTP, le message d'erreur d'un fournisseur d'IA ou du système : l'app ne le traduit pas, elle le cite, introduit par une phrase dans la langue de l'utilisateur.
3. **L'interface suit `uiLocale`, un mail suit la langue du mail.** Ce que NiceMailer écrit dans un mail envoyé est lu par le destinataire, pas par l'utilisateur.
4. **Ce qui ne peut pas être vérifié par un test ne tient pas.** Chaque règle a son test, qui échoue sur le prochain écart.

## 3. Chantiers, dans l'ordre

### 3.1 Le noyau décrit, l'app écrit

C'est le premier chantier, parce qu'il change le contrat : plus tard, il coûte plus cher (clients, MCP, documentation, tests). Il s'est fait en deux lots : les erreurs des requêtes, puis le texte des événements et des résultats.

**Constat.**

| Ce que le noyau envoie | Où l'app l'affiche |
| --- | --- |
| `RpcError.message` : 108 constructions dans `crates/`, en anglais (4 en français), souvent composées par `format!` (« account {key} is not synchronized ») | `AppModel` (`lastError = error.localizedDescription`, qui vaut « message (code N) »), `AppModel+Compose` (`Cannot send: %@`, et le message seul par défaut), `AccountSetup` (refus du mot de passe, connexion IMAP impossible, toute autre erreur), `AI.errorFlash` (`AI: %@`) |
| `ActionReverted.reason` | barre d'état : « … · undone: %@ » |
| `SendFailed.reason`, `SendPartial.refused` | barre d'état et composeur |
| `AiFailed.message` (« dit quoi faire ») | panneau de l'IA |
| Le message d'une maintenance (`Maintenance.message`, `docs/updates.md`) | écran sans noyau |
| La sortie d'erreur du CLI (`MailboxCheck.errorMessage`) | « Change Mailbox… » |

Une partie de ces textes est du noyau (« the draft changed since: save it and run again »), une partie vient d'un tiers (la réponse du serveur dans `reason`). Aujourd'hui les deux sont mélangés dans la même chaîne.

**Décision.**

1. Chaque erreur du contrat porte, en plus de `code` (la famille, inchangée), une **raison** stable et ses **valeurs** dans `data` : `{"reason": "account_not_synchronized", "account": "perso"}`. `message` reste, en anglais, pour les journaux et le CLI ; aucun client ne l'affiche à un utilisateur.
2. Les événements qui portent du texte le séparent de même : `ActionReverted { …, reason: "server_refused", server_text: "…" }`, `SendFailed { …, reason, server_text }`, `AiFailed { code, provider_text }`. `server_text` et `provider_text` sont les textes de tiers, cités tels quels (principe 2).
3. Dans l'app, une seule fonction traduit une erreur du noyau : la raison choisit la clé du catalogue, les valeurs en sont les arguments.
4. Le CLI garde ses textes anglais et peut les construire des mêmes raisons, pour qu'un message du CLI et de l'app disent la même chose.
5. **Le serveur MCP transmet la raison.** Aujourd'hui `nicemailer-mcp` réduit une erreur du noyau à une phrase anglaise (`ToolError`, `nm-mcp/src/tools.rs`), choisie par la famille (`code`). Le résultat d'erreur d'un outil (`isError: true`) garde cette phrase, que l'agent lit, et porte en plus la raison et ses valeurs, dans un second bloc de `content`, en JSON (`{"reason": "draft_by_user", "draft": 12}`) : un agent peut décider sur la raison (relire un brouillon, attendre un corps en téléchargement, prévenir l'utilisateur) sans analyser un texte. Les textes de tiers lui parviennent aussi, séparés. Pas en `structuredContent` : un client peut le vérifier contre le schéma de sortie de l'outil, qu'une erreur ne suit pas, et le modèle ne lit que `content`. Les phrases de `ToolError` restent choisies par famille là où elles disent à l'agent quoi faire (relire le brouillon) ; ailleurs, c'est la phrase anglaise de la raison. `docs/mcp.md` décrit le format.
6. **Pas de décalage de versions.** L'app, le noyau, le CLI et `nicemailer-mcp` sont livrés ensemble dans `NiceMailer.app` et mis à jour ensemble (`mise-a-jour.md`) : aucun client n'est plus ancien que le noyau qu'il interroge. Une raison que l'app ne connaît pas n'est donc pas un cas de compatibilité, c'est un oubli, que les tests ci-dessous refusent. La liste des raisons n'est pas versionnée. Seul garde-fou, pour qu'un oubli passé entre les tests ne laisse ni écran vide ni plantage : une phrase générique avec le `message` anglais, journalisée comme une erreur.

**Lot 1, livré : les erreurs des requêtes.**

- `nm_api::reason` : 103 raisons, chacune avec sa phrase anglaise à valeurs nommées (`{tag}`), déclarées une fois (`reasons!`) ; `RpcError::new(code, raison)` construit `message` depuis la phrase et `data` depuis la raison et ses valeurs. Le store (`StoreError::Invalid`, `InvalidMove`, `CannotSend`), les refus du trousseau (`Refusal::why`), les erreurs de l'IA (`AiError::why`, `BadInput` avec sa raison) et les fichiers illisibles (`Unreadable::why`) portent leur raison jusqu'au contrat. Les messages du store qui étaient en français sont en anglais.
- Les erreurs de programmation (`invalid_params`, `not_found`, `internal`…) ont une raison générique et un `detail` anglais ; l'app les dit d'une phrase générique.
- L'app : `CoreReason.swift` écrit chaque raison (85 phrases, anglais et français) ; `RPCError.localizedDescription` est cette phrase. Les valeurs de `data.reason` du test de compte, `auth` et `connect`, s'appellent `imap_auth_refused` et `imap_unreachable`.
- Le serveur MCP transmet la raison (point 5).
- Le CLI affiche toujours `message`, désormais tiré de la raison : rien à changer de son côté.

**Écart avec le plan de tests.** Le plan voulait un test qui provoque chaque raison. Il n'existe pas : à la place, `contract_doc.rs` vérifie que chaque raison est donnée par le code (une raison oubliée échoue), et les tests existants provoquent les raisons qu'ils couvraient déjà par leur texte (désormais par leur raison). Une raison donnée au mauvais endroit, ou avec une valeur manquante, n'est donc pas détectée partout ; `every_template_is_completed_by_its_values` vérifie seulement que les valeurs déclarées complètent la phrase.

**Lot 2, livré : le texte des événements et des résultats.** `AccountError` et les comptes de `list_accounts`, `ActionReverted`, `SendFailed` et `AiFailed` portent `why`, le même objet que `data` d'une erreur, à côté de leur texte anglais (`message`, `error`) ; l'app l'écrit (`CoreReason.text(why:message:)`). Raisons ajoutées : `server_refused_change`, `attempts_failed`, `smtp_refused`, `ai_cancelled`. Une connexion qui échoue pendant la synchronisation est dite `imap_failed` avec le texte de l'erreur cité, qu'elle vienne du serveur ou du trousseau : l'imprécision est assumée, le texte cité dit la cause.

Restent en anglais, cités comme des détails techniques dans une phrase de l'app : `SendPartial.refused` (l'adresse et la réponse du serveur), `ConfigProblem` (l'erreur de l'analyseur TOML), `StartupError.message` (un noyau qui n'a pas démarré), `mx_error`, l'échec d'un désabonnement en un clic, l'erreur du CLI lue par « Change Mailbox… », et l'erreur d'une exécution d'IA relue par `ai_get_run` (`AiRunView.error`, enregistrée en texte dans `ai_run` ; la dire par sa raison demanderait de l'y enregistrer, une migration, pour un cas que l'app ne voit qu'après avoir manqué l'événement `AiFailed`).

**Inventaire et ordre.** Toutes les erreurs ne sont pas vues par un utilisateur : `INVALID_PARAMS` et `METHOD_NOT_FOUND` sont des erreurs de programmation, qui restent en anglais dans les journaux et dans une phrase générique à l'écran. L'inventaire classe les 108 erreurs en trois groupes : vue par l'utilisateur (une raison, une clé), erreur de programmation (`message` seulement), texte de tiers (`server_text`). On commence par les erreurs affichées par l'app aujourd'hui (tableau ci-dessus).

**Tests.**

- Rust : chaque raison est une constante de `nm_api::reason` (`RpcError::new` n'accepte pas de texte libre) ; `contract_doc.rs` vérifie que la table de `docs/contract.md`, « Errors », est exactement le registre, valeurs comprises, et que chaque raison est donnée par le code ; `nm-api` vérifie que chaque phrase est complétée par ses valeurs.
- Swift : `LocalizationTests` lit la même table et vérifie que chaque raison est écrite par `CoreReason` ou déclarée générique, jamais les deux, que chaque phrase ne lit que les valeurs de sa raison, et que les arguments des traductions sont ceux de leur clé.
- MCP : un refus du noyau garde sa raison (`nm-mcp`), et un agent la reçoit dans le second bloc (`mcp_proxy.rs`).

### 3.2 Le texte écrit dans les mails

**Décidé et livré.** La ligne qui introduit une citation (« Le 7 nov. 2026 à 10:12, Alice a écrit : ») et l'en-tête d'un transfert gardent la langue de l'app : simple et prévisible, au prix d'une ligne française au-dessus d'une réponse anglaise écrite depuis une interface française. Leur date suit `uiLocale`, comme l'interface : la région et l'horloge du Mac (une interface anglaise sur un Mac réglé en France écrit « 7 Nov 2026 at 21:13 », plus « Nov 7, 2026 at 9:13 PM »). Le PDF de preuve (`MailPDF`) suivait déjà `uiLocale`. Écartés : la langue détectée du mail répondu (une détection se trompe sur un mail court) et un réglage par compte (un réglage de plus). Les préfixes de sujet `Re:` et `Fwd:` restent anglais : c'est l'usage de la RFC 5322, et les clients les reconnaissent dans toutes les langues.

### 3.3 Les citations dans toutes les langues

**Livré.** Le constat de ce plan se trompait d'endroit : `nm-core/src/segments.rs` (le texte de l'utilisateur dans un brouillon) reconnaît la ligne qui introduit une citation à sa forme, en toute langue. La détection qui dépendait de la langue est celle des mails reçus, `nm-store/src/clean.rs` (l'historique replié de l'affichage simplifié, `docs/thread-display.md`). Elle connaissait l'anglais, le français, l'espagnol, l'italien, et l'allemand de Thunderbird seulement (« schrieb: » en fin de ligne, quand Gmail et Apple Mail écrivent « schrieb Alice <a@x>: »).

Elle a maintenant une table de formulations (`WORDINGS`) : anglais, français, allemand, espagnol, italien, portugais, néerlandais (deux formes), suédois, danois, norvégien, polonais (deux formes), russe, chinois ; et, pour les clients qui n'écrivent aucun mot (Gmail en japonais ou en russe : « 2026年11月3日(火) 10:52 Alice <a@x>: »), une règle de forme : une ligne finie par deux-points, qui contient une adresse et un chiffre. Cette règle ne s'applique qu'à la ligne juste au-dessus de l'historique cité final, et une phrase de l'utilisateur finie par deux-points n'est prise que si elle contient une adresse. Les en-têtes à la Outlook, les séparateurs « message d'origine », les marques de transfert et les mentions (« Von meinem iPhone gesendet ») suivent les mêmes langues. Un test par forme ; le découpage se fait à la lecture, les mails déjà reçus en profitent sans réanalyse.

Pas fait : la mesure sur un échantillon de vrais mails, faute d'échantillon. Les formes de la table viennent des formats connus des clients courants ; une forme oubliée laisse l'historique visible, sans rien casser.

### 3.4 Ce que l'utilisateur tape

**Décidé et livré : l'ordre de la région.** Une date tapée dans le sélecteur d'attributs (`ui.md`, « Sélecteur d'attributs ») se lit dans l'ordre de la date courte de la région du Mac, quelle que soit la langue : `1/10/2026` est le 1er octobre en France, le 10 janvier aux États-Unis ; `2026/10/1` au Japon ; `2026-10-01` partout. Six chiffres se lisent d'abord dans cet ordre, puis année d'abord, et chaque ligne dit sa lecture (« mm jj aa »). L'aide du panneau donne ses exemples dans l'ordre de la région.

La colonne de date des listes geek affiche la même date courte, deux chiffres par champ, pour qu'on lise ce qu'on tape : le gabarit `ddMMyy` qu'elle utilisait suit la langue dans ICU, pas la région, et une interface française sur un Mac américain aurait affiché le jour d'abord tout en lisant le mois d'abord.

Reste : les unités de durée (`10j`, `3w`) ne connaissent que l'anglais et le français ; chaque langue ajoutée y entre (section 3.9), en vérifiant qu'aucune lettre ne désigne deux unités.

### 3.5 Les raccourcis et les dispositions de clavier

Le thème geek se pilote par des touches simples (`;`, `/`, `@`, `#`, `[`…) et l'app a `⌘+`, `⌘-`. Sur un clavier allemand, nordique ou suisse, certaines demandent ⌥ ou ⇧, d'autres n'existent pas. À faire : relever chaque raccourci, le jouer sur les dispositions courantes (US, français, allemand, espagnol, italien, suisse, nordiques, japonais), et choisir, par raccourci, de le garder par caractère, de lui donner une seconde touche, ou de le lire par position de touche. Le scénario UI peut fixer une disposition comme il fixe la langue.

### 3.6 Les nombres et les calendriers

- Les nombres insérés par `%lld` ne sont pas groupés (« 12345 mails »). Les comptes qui peuvent dépasser 999 passent par un `NumberFormatter` de `uiLocale`, insérés en `%@`, en gardant la variation de pluriel sur le nombre.
- `uiLocale` prend le calendrier du Mac : un utilisateur réglé sur un calendrier non grégorien (hégirien, bouddhiste, japonais) voit ses dates dans ce calendrier. À vérifier écran par écran, en particulier le calcul « aujourd'hui / cette année » et les dates tapées (3.4).

### 3.7 Les textes hors du catalogue

- `InfoPlist.strings` (nom de l'app, textes des autorisations) n'existe qu'en français, écrit par `scripts/make-bundle.sh` : à générer pour chaque langue, depuis le catalogue.
- Les menus de macOS fournis par AppKit sont traduits par le système, à condition que `CFBundleLocalizations` liste la langue (`make-bundle.sh`).
- Les libellés d'accessibilité (VoiceOver) et les bulles d'aide (`.help`) passent par `L()` ; le test des textes en dur dans les vues les couvre déjà, il faut l'étendre à `accessibilityLabel`.
- Le séparateur de liste « , » des destinataires (`Compose.recipients`) et des tags : `ListFormatter`, ou une clé du catalogue.

### 3.8 La longueur et l'écriture de droite à gauche

- Une traduction allemande est environ 30 % plus longue que l'anglais. Les colonnes de largeur fixe du thème geek (date, expéditeur), la barre d'état et les panneaux sont à essayer avec la pseudo-langue « double longueur ».
- L'arabe et l'hébreu demandent la mise en page miroir, le texte bidirectionnel (un sujet hébreu dans une interface anglaise) et des colonnes en caractères de largeur fixe qui n'ont plus de sens. C'est un projet à part entière : ne pas l'annoncer sans l'avoir décidé.

### 3.9 Ajouter une langue

La procédure, à écrire dans `docs/localization.md` quand les chantiers 3.1 à 3.7 sont livrés :

1. La langue entre dans `CFBundleLocalizations` (`make-bundle.sh`) et dans la liste des langues vérifiées par les tests.
2. Chaque clé du catalogue reçoit sa traduction, ses variations de pluriel selon les catégories de la langue (le russe et le polonais en ont 3 ou 4, l'arabe 6).
3. Un scénario UI dans cette langue (`fixtures/<scenario>.language`).
4. Une relecture par une personne dont c'est la langue : une traduction automatique non relue se voit.
5. Une ligne au `CHANGELOG.md`.

## 4. Critère de fin

Une langue s'ajoute sans toucher au code de l'app ni du noyau : un fichier de traduction, une relecture, un scénario. Tant que ce n'est pas vrai, ce plan n'est pas terminé.
