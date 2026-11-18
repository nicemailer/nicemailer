Il y a trois versions ou thèmes pour l'application Swift NiceMailer : classic, design et geek.

Le thème classique s'approche des canons historiques du design d'une application macOS et est destiné à faciliter la transition depuis une application telle que Apple Mail. 

Le thème design cherche à offrir une interface sobre et élégante privilégiant la lisibilité.  Elle est conçue pour les utilisateurs privilégiant l'esthétique. 

Le thème geek est une interface minimaliste conçue comme un IDE de programmation.


Quelques exemples de différences :

- la couleur de fond et les arrondis délimitant les zones de l’interface peuvent être marqués dans la version classique, sobres dans la version design et inexistante dans la version geek.
- les ascenseurs sont standard dans la version classique, plus sobres dans la version design et inexistante dans la version geek
- La version classique à une barre d’outils en haut de l’interface avec des icônes de boutons alors que dans la version geek, c'est une barre de statut plus compacte, qui peut être en bas, en haut ou masquée. Dans cette version geek, chaque élément présent dans la barre de statut peut être ouvert avec un raccourci clavier d'une seule touche et affiche alors un menu contextuel dans le même style que Helix-editor.
- Dans la version classique, l'animation pour révéler la barre de gauche avec les différents dossiers est clairement visible. Elle est très rapide et subtile sur la version design et absolument inexistante sur la version geek où aucune milliseconde n'est perdue. 

Pendant le développement, il y a un menu thème qui permet de basculer d'un thème à l'autre. Ce sera plus tard une option de configuration. 

État (3 novembre 2026) : le thème geek est implémenté ; classic correspond à la première interface ; design n'existe pas encore et affiche classic.

Les touches citées dans ce document sont celles par défaut ; l'utilisateur peut les redéfinir dans `keymap.toml` (`raccourcis.md`).

### Préférences

Ajouté en novembre 2026. Le menu de l'app « Réglages… » (⌘,) ouvre les Préférences, communes aux trois thèmes, avec les contrôles standard du Mac : une fenêtre de réglages n'est pas le lieu des touches du thème geek. Un onglet par sujet ; le premier est **Comptes** (`docs/accounts.md`, « In the app »).

- **Liste des comptes**, dans l'ordre d'affichage (celui du sélecteur `@` et de la colonne des dossiers), avec ce qui ne va pas (désactivé, connexion impossible, archive d'un import mbox) ou le fait qu'il soit seulement regardé. Un glisser-déposer ou les flèches ↑ ↓ sous la liste changent l'ordre ; la liste le montre aussitôt et l'envoie au noyau.
- **Ajout d'un compte** : trois champs seulement, l'adresse, le mot de passe et le nom affiché dans « De ». Le nom est repris du premier compte qui en a un, dans l'ordre d'affichage, tant que l'utilisateur n'a pas tapé le sien. Le noyau trouve les serveurs (par le domaine, sinon par son MX) ; d'autres champs n'apparaissent, sous les trois premiers et avec leur raison, que si l'essai le demande : le serveur IMAP quand ni le domaine ni le MX ne le donnent (ou qu'il ne répond pas) ; le mot de passe du serveur SMTP quand il refuse celui de la boîte (mot de passe d'application) ou qu'il appartient à un autre fournisseur (le mot de passe de la boîte ne lui est jamais envoyé) ; le serveur SMTP quand aucun ne répond, ou l'ajout « sans envoi pour l'instant » ; l'écran d'import (`import-initial.md`, section 3) quand l'Inbox dépasse les seuils, avec les dates par défaut. La clé du compte n'est jamais demandée. Chaque étape réussie est gardée : un nouvel appui sur « Ajouter le compte » reprend là où l'essai s'était arrêté.
- **Sur le serveur** : une fois les serveurs trouvés, et avant que rien ne soit écrit, une section dit toujours ce que le compte va changer sur le serveur : les dossiers qui seront créés, puis le dossier de chaque rôle du workflow, modifiable (`dossiers.md`, « A la création du compte IMAP dans niceMailer »). L'écran d'import vient entre la phrase et les dossiers. Un même dossier choisi pour deux rôles est refusé. Un interrupteur, désactivé par défaut, permet de **seulement regarder pour l'instant** : le compte est ajouté en lecture seule, sans dossier créé ni mail déplacé, marqué lu ou envoyé (`dossiers.md`, « Regarder avant d'organiser »).
- **Organiser un compte seulement regardé** : sa page l'explique et propose « Organiser… », qui montre la même section (sans l'interrupteur) et l'écran d'import, puis applique le tout.

### Thème geek : choix retenus

- Police monospace pour l'interface (SF Mono) ; corps des mails en **iA Writer Mono V**, livrée avec l'app (licence SIL OFL).
- Palette inspirée de Helix, en variante claire et sombre selon l'apparence de macOS.
- HTML de l'expéditeur : sa mise en page et son CSS sont gardés, assainis par le noyau (aucune adresse distante chargeable, rien qui dépende du survol ou de la taille de l'écran), chaque message dans un cadre isolé : son style ne touche ni les autres messages ni l'interface. Un mail qui prévoit son mode sombre s'affiche avec ses couleurs, sombres dans le thème sombre. Les autres, faits pour une page blanche, sont adaptés au thème sombre par Dark Reader (licence MIT) : fond du thème, texte clair, teintes des boutons et des titres gardées ; il tourne isolé de la page, sans aucun accès au réseau, et s'efface dans le thème clair. Les polices web ne sont pas chargées. Les images intégrées au mail (`cid:`) s'affichent en html comme en images : elles ne demandent rien au réseau.
- Corps des mails en texte par défaut. Un sélecteur `text | html | images` en haut du mail, et `vh`, basculent en HTML assaini ; `images` (ou `vi`) charge aussi les images distantes (voir « Images distantes »). Le choix est mémorisé **par adresse d'expéditeur** (`address.body_format`, `address.remote_images`, `schema.sql`).
- Compte unifié **all** : avec plusieurs comptes, l'app s'ouvre sur « all », qui regroupe tous les comptes. Chaque dossier (Inbox, Archive…) y réunit les dossiers de même rôle de tous les comptes, avec leurs compteurs additionnés. La colonne des dossiers n'a qu'un bloc, intitulé « all » ou le nom du compte choisi ; le sélecteur de dossiers et le menu `;` ne montrent que les dossiers de cette vue, sans notion de compte.
- Barre de statut : le mode (l'icône de l'application en mode normal ; sinon le mode écrit : `EDT` pendant l'écriture d'une réponse, le préfixe en attente comme dans Helix, `PCK` dans un sélecteur, `ASK` pour une question), puis le compte (icône @, « all » pour la vue unifiée) et le dossier (icône de dossier), chacun suivi du rappel de sa touche. Chaque élément s'ouvre d'une touche ou d'un clic : le mode avec `?` (liste des touches), le compte avec `@` (sélecteur de comptes, « all » en tête : on arrive dans le dossier de même rôle, sinon l'Inbox), le dossier avec `;` (menu des dossiers de la vue, d'« Everything » et des écrans, qui se déroule à gauche depuis le bloc dossier de la barre de statut : en dessous si la barre est en haut, au-dessus si elle est en bas ; ses lignes se cliquent aussi). Les menus `m` (déplacer), `s` (enregistrer) et `v` (affichage) ont aussi leur bloc dans la barre de statut, une icône suivie de la touche. L'IA aussi (novembre 2026) : un bloc après `s`, des étincelles suivies de `a`, ou de `⌃I` pendant l'écriture d'une réponse ; un clic ouvre la palette IA comme la touche (`ia.md`, section 11), et ne fait rien pendant qu'un résultat de l'IA est affiché. Un large espace sépare trois groupes : ce que la liste affiche (`@`, `;`, `f` et les pastilles), la préférence d'affichage (`v`), les actions sur le mail courant (`m`, `s`, IA). Leur menu se déroule depuis ce bloc de la même façon. À l'extrémité droite de la barre, sur tous les écrans, un bloc `?` (point d'interrogation cerclé) ouvre la liste des touches comme la touche `?` ; un second clic la ferme. Il rappelle l'aide à qui ne connaît pas encore la touche. Dans un menu, les flèches surlignent une ligne (bas commence à la première, haut à la dernière, la liste tourne) et entrée va à la ligne surlignée ; une touche agit toujours directement. Barre masquée : les menus s'affichent en bas à droite. `h` ne fait plus rien : il faisait double emploi avec `;`. Pendant qu'un tag, un groupe ou un contact est listé, la barre l'affiche à la place. Les messages (résultat d'une action, erreur) s'affichent à droite et disparaissent à la touche suivante. Un déplacement nomme le mail et le dossier : « Devis toiture » déplacé dans Archive (sujet coupé à 50 caractères) ; pour plusieurs mails, leur nombre : 3 conversations déplacées dans Archive.
- Les sélecteurs (`@`, `l`, `t`, `g`, `f`) sont des listes filtrables à la manière de Helix : on tape pour filtrer (chaque mot tapé doit apparaître), `ctrl-n` / `ctrl-p` ou les flèches pour se déplacer, entrée pour choisir, échap pour fermer. À l'ouverture, la première ligne est sélectionnée ; la liste tourne : flèche haut (ou `ctrl-p`) depuis la première ligne sélectionne la dernière, flèche bas (ou `ctrl-n`) depuis la dernière revient à la première. Pour `t` et `g`, taper un nom absent propose de le créer. Le texte tapé s'édite comme dans tout champ macOS : flèches gauche et droite pour déplacer le curseur (⌥ par mot, ⌘ jusqu'au bout), effacement des deux côtés du curseur, et ⌘V colle le texte copié dans une autre app, sur une seule ligne, sans les espaces autour. Un clic en dehors d'un sélecteur, d'un menu ou de l'aide le ferme, comme échap, sans rien faire d'autre.

L'interface utilisateur de NiceMailer est, quel que soit le thème choisi, minimaliste et sobre. 

La fenêtre principale permet de lister les mails en deux colonnes : 
- A gauche, sur un petit tiers de l'écran, mais resizable, la liste des mails. 
- A droite, sur le reste de l'écran, l'affichage du mail sélectionné à gauche. 

Lors de l'ouverture d'une fenêtre, le premier mail est systématiquement sélectionné dans la liste, de sorte que la partie de droite affiche quelque chose. 

### Fenêtres

État (17 novembre 2026) : **une seule fenêtre**. Toutes les fenêtres partageaient le même modèle (`AppModel`) : dossier affiché, mail sélectionné, recherche, réponse en cours. Une deuxième fenêtre n'était qu'un miroir de la première, et son ouverture reconnectait l'app au noyau. Fichier > Nouvelle fenêtre (⌘N) et les onglets de macOS sont donc retirés (`SingleWindow`), jusqu'à ce que chaque fenêtre ait son propre contexte, décrit ci-dessous.

**Une fenêtre, un contexte.** Chaque fenêtre est une lecture indépendante de la même boîte. Lui appartiennent :
- ce que la liste montre : la vue (« all » ou un compte), le dossier ou le dossier virtuel, la liste d'un tag, d'un groupe ou d'un contact, la recherche, les attributs (`f`), le tri ;
- le mail sélectionné, la lecture (message atteint, texte ou HTML de cet affichage, défilement), les mails marqués (`J` `K`) ;
- l'écran (mails, contacts, historique, tags, groupes) et son filtre, le menu ou le sélecteur ouvert, la question en attente ;
- la réponse en cours d'écriture et l'IA qui y travaille, le tri par l'IA en cours (`a` `s`) ;
- le message de la barre de statut ;
- la pile d'annulation : `z` défait la dernière action faite **dans cette fenêtre** (comme Mail ; une pile commune ferait défaire, depuis une fenêtre, une action qu'on n'y voit pas).

Sont communs à toute l'app :
- la connexion au noyau (une seule, quel que soit le nombre de fenêtres), son état (démarrage, maintenance, mise à jour, arrêt par le menu), la boîte utilisée ;
- la liste des dossiers et leurs compteurs, les comptes, les adresses d'envoi, l'état de l'IA ;
- les réglages (`config.toml`, `[ui]`), le thème, la taille du texte, les choix mémorisés par expéditeur (texte ou HTML, images distantes), l'accès au carnet d'adresses.

**Cohérence.** Les fenêtres ne se parlent pas. Une action faite dans l'une (déplacement, lu, drapeau, tag) atteint les autres par les événements du noyau, comme une action du CLI, d'un agent MCP ou du téléphone (`evenements.md`) : chaque fenêtre recharge ce qu'elle affiche. Le mail sélectionné qu'une autre fenêtre a déplacé suit donc la même règle que s'il l'avait été par le téléphone. La connexion reçoit les événements une seule fois et les remet à chaque fenêtre.

**Ouvrir une fenêtre.** Fichier > Nouvelle fenêtre (⌘N ; la touche `c` du thème geek reste celle du nouveau mail). La nouvelle fenêtre s'ouvre sur la vue et le dossier de la fenêtre au premier plan, avec la sélection d'une ouverture : le premier mail. Elle ne recopie ni la recherche, ni les marques, ni la réponse en cours. Les onglets de macOS sont acceptés : un onglet est une fenêtre.

**Clavier et menus.** Les touches vont à la fenêtre clé seulement. Les commandes de menu qui portent sur ce qu'on voit (Recharger, Présentation) visent la fenêtre au premier plan. Celles qui portent sur toute l'app (Changer de boîte…, Agents IA, Thème, taille du texte) valent pour toutes les fenêtres : changer de boîte recharge chacune.

**Liens vers un mail** (`nicemailer://m/<id>`, `docs/deep-links.md`) : ouverts dans la fenêtre au premier plan ; sans fenêtre, dans une nouvelle.

**Rédaction.** Une réponse appartient à sa fenêtre : chaque fenêtre a son propre éditeur. Un brouillon ne s'ouvre que dans une fenêtre à la fois : le reprendre (`r`) depuis une autre amène au premier plan celle qui l'a ouvert. Le noyau protège de toute façon le brouillon par sa révision (`brouillons.md`, `update_draft`). Fermer une fenêtre dont la réponse est en cours la garde, comme Échap, et vaut fermeture de la fenêtre de rédaction : le brouillon part au serveur (`brouillons.md`).

**Fermeture et relancement.** Fermer la dernière fenêtre quitte l'app, comme aujourd'hui. Au relancement, les fenêtres rouvrent à leur place, chacune sur sa vue et son dossier, avec la sélection d'une ouverture ; recherche, marques et menus ne sont pas restaurés.

**Préférences d'affichage.** Le nombre de colonnes, la position de la barre de statut et la largeur de la liste sont propres à chaque fenêtre. La dernière modifiée sert de défaut à la fenêtre suivante.

**Thèmes.** Ces règles valent pour tous les thèmes : le contexte est dans le modèle, pas dans les vues.

**Vitesse.** Une fenêtre de plus ne coûte qu'une requête paginée par rechargement (`architecture.md`, « Pagination partout ») : ni seconde connexion, ni seconde copie des dossiers. Une fenêtre réduite dans le Dock ou cachée peut attendre d'être de nouveau visible pour recharger.

**Tests.** Tests unitaires : deux fenêtres sur un même faux noyau (`FakeCore`), sélections et dossiers indépendants, un événement remis aux deux, `z` propre à chacune. Scénario d'interface : une deuxième fenêtre ouverte, chacune sur un dossier, une action dans la première visible dans la seconde, les touches de l'une sans effet sur l'autre, un brouillon refusé dans la seconde fenêtre.

Questions ouvertes :
- ⌘N pour une nouvelle fenêtre, ou pour un nouveau mail comme Mail (la touche `c` existe déjà dans le thème geek, pas dans le thème classique) ?
- Le thème classique doit-il avoir le multi-fenêtre dès la première version, ou le thème geek d'abord ?
- Faut-il, à terme, un panneau divisé dans une même fenêtre (deux listes côte à côte) ? Il reposerait sur le même contexte par liste.

### Liste

- Chaque ligne : le point des non lus, la date, l'expéditeur, un trombone s'il y a des pièces jointes, le sujet et ses tags, (dans Everything) le dossier, le nombre de messages de la conversation, le drapeau.
- La colonne de l'expéditeur nomme **la personne avec qui l'on correspond**, pas l'auteur du dernier mail (novembre 2026) : dès qu'on avait répondu, l'auteur du dernier mail était soi-même, ce qui ne disait rien. Elle montre le dernier auteur qui n'est pas soi ; pour un mail qu'on a écrit seul, son premier destinataire (To avant Cc). Une flèche grisée **→** devant le nom dit qu'on a écrit le dernier mail, et que la conversation attend l'autre : « → Alice ». Un brouillon non envoyé ne compte pas : une réponse en cours d'écriture laisse la ligne comme elle était. Un mail est le nôtre selon la règle des en-têtes allégés de la lecture : son expéditeur est une des adresses de la boîte, ou il est dans Sent ou Drafts. C'est le noyau qui le décide, et l'app qui l'affiche.
- Le trombone est seul, sans chiffre, et placé avant le sujet : à côté du nombre de messages en fin de ligne, « 📎 3 » se lirait comme trois pièces jointes.
- Le drapeau est une icône orange en fin de ligne, couleur propre, distincte du point jaune des non lus.
- ⌘A marque tous les mails de la liste telle qu'affichée (les lignes chargées jusqu'ici), pour une action groupée ; la barre de statut dit combien ; Échap les démarque. Seulement sur l'écran des mails, sans panneau ouvert.

#### Actions groupées

Une action sur plusieurs mails (les mails marqués) part en **une seule** demande au noyau, quelle que soit leur nombre : lu et non lu (`u`, `U`), drapeau (`*`), tag (`t`), déplacement (`d`, `m`, Suppr, les cibles directes de Quarantine). Le noyau les traite dans une seule transaction et n'émet qu'un lot d'événements. La liste change d'un coup : lu, non lu et drapeau s'affichent tout de suite sur toutes les lignes, avant la réponse du noyau, et les mails déplacés quittent la liste ensemble ; en cas d'erreur, la liste est relue et montre l'état du noyau. Avant (novembre 2026), l'app envoyait une demande par mail, et les lignes changeaient l'une après l'autre. Un déplacement groupé s'annule d'un seul `z`, en une demande. Un mail dont l'expéditeur est suspect d'usurpation n'est pas déplacé avec les autres : il revient dans la liste et l'app pose la question pour lui seul ; un mail pour lequel la cible n'est pas permise reste aussi où il est.

### Affichage d'une conversation

L'affichage est simplifié au maximum : chaque message montre ce qu'il apporte. Le noyau découpe chaque message et l'app n'affiche que le résultat (détail : `docs/thread-display.md`).

- Masqués par défaut : la citation finale (historique cité, avec sa ligne « … a écrit : » ou le bloc d'en-têtes d'Outlook), le texte qui recopie un message précédent, les signatures (après `-- `, ou lignes finales répétées par un expéditeur), les mentions (« Envoyé de mon iPhone », avis de confidentialité). Chaque partie masquée est remplacée par une ligne « ··· » cliquable.
- Les citations intercalées entre les réponses restent visibles, atténuées, avec une barre par niveau. Un message transféré n'est jamais masqué.
- Comme dans Gmail, le dernier message et les non-lus sont dépliés ; les autres tiennent sur une ligne (expéditeur, date, début du texte). Un message qui ne fait que citer l'indique.
- En-têtes allégés : expéditeur et date ; les destinataires seulement s'ils changent d'un message à l'autre, **toujours pour un mail seul dans sa conversation** (ajouté en novembre 2026 : sans message précédent, rien ne dit à qui il est parti, pas même quand c'est à soi seul), et **toujours pour un mail que l'on a écrit** (ajouté en novembre 2026) : son expéditeur, c'est soi, et l'information est à qui il est parti — sans quoi un mail envoyé à une seule personne n'affiche personne. Un mail est le nôtre quand son expéditeur est une des adresses de la boîte (celles de `accounts.toml`, que le noyau reçoit du démon, et celles des comptes d'archives) ou qu'il est dans Sent ou Drafts ; il le reste donc une fois rangé dans Waiting ou Todo (`reponse.md`, section 8).
- `T` (comme dans mutt) affiche ou masque tout ce que la vue simplifiée cache, pour la conversation ouverte ; `S` va au texte qui suit la citation suivante ; `ctrl-k` remonte au message précédent de la conversation et `ctrl-j` redescend au suivant, un seul déplié à la fois : le message courant se replie, l'autre se déplie et la lecture va à son en-tête (au départ, le dernier message déplié, ou le dernier en-tête cliqué ; arrêt aux deux bouts, que la barre de statut signale ; avec `T`, rien ne se replie, on passe d'un en-tête à l'autre). Un clic sur un en-tête plie ou déplie le message, un clic sur « ··· » affiche cette partie.

### Tags et groupes dans l'en-tête

Ajouté en novembre 2026. La ligne du sujet, en haut du mail affiché, nomme après le sujet les tags de la conversation (`#Urgent`, comme dans la liste) et les groupes de la personne avec qui elle est (`@VIP`, comme dans la colonne des dossiers), dans la couleur d'accent. Cette personne est celle que nomme la colonne De de la liste et que `g` ajoute (`groupes.md`, section 6) : l'en-tête nomme ainsi les groupes dont `g` peut la retirer, jamais ceux d'un autre participant. Quand le sujet ne tient pas à côté, les tags et groupes ouvrent la seconde ligne, devant les contrôles. Ils ne sont jamais coupés. Le noyau les donne avec la conversation (`get_thread`) ; un ajout ou un retrait les met à jour aussitôt.

### Ajouter et retirer un tag ou un groupe

Ajouté en novembre 2026. Les sélecteurs `t` et `g` proposent chaque action sur sa propre ligne, pour le mail courant comme pour les mails marqués. Une sélection peut mêler des mails qui ont un tag et d'autres qui ne l'ont pas : une ligne qui ajoute ou retire selon le cas ne dirait pas ce qu'elle fait.

- D'abord, marquées `−`, les lignes « retirer VIP » : un tag que porte au moins un des mails visés, ou un groupe où est la personne d'au moins l'un d'eux. Le retrait ne touche que les mails qui l'ont.
- Ensuite, les lignes d'ajout : les tags (ou groupes) existants que tous les mails visés n'ont pas encore. Les tags sont ceux de tous les comptes, une ligne par nom : un tag appartient à un compte (mot-clé IMAP de sa boîte), mais l'utilisateur le pense par son nom, comme `;T` qui liste les mails des tags de même nom de tous les comptes ; choisi, il est créé dans le compte du mail. Avant (novembre 2026), seuls les tags du compte du mail étaient proposés : avec plusieurs comptes, la liste était souvent vide.
- Un nom porté par une partie des mails a donc les deux lignes. Avec plusieurs mails, chaque ligne dit combien l'ont : « 1 sur 2 ».
- Un nom tapé qui n'existe pas donne la ligne `+`, qui le crée et l'ajoute, comme avant.

Pour un seul mail, cela revient à : ses tags (ou les groupes de sa personne) à retirer, puis les autres à ajouter. Pour connaître les groupes des personnes de mails marqués, l'app les demande au noyau à l'ouverture de `g` (`thread_labels`, 1 000 conversations au plus) ; tant qu'il n'a pas répondu, ceux de l'en-tête valent pour le mail ouvert. Avant (novembre 2026), `t` sur un tag que tous les mails visés avaient le retirait, sans que rien ne l'annonce, et `g` ne faisait qu'ajouter.

### Pièces jointes

- Au pied de chaque mail déplié, une dalle par pièce jointe : l'icône de son type (SF Symbol, aux couleurs du thème : image, PDF, texte, tableur, présentation, archive, audio, vidéo, code, agenda, contact, mail, fichier), le nom coupé au milieu pour garder l'extension, la taille. 3 à 5 dalles par ligne selon la largeur du panneau. Un mail plié montre un trombone et le nombre de pièces jointes.
- Un clic enregistre le fichier dans Téléchargements, sans jamais écraser un fichier existant (« devis (2).pdf ») ; la barre de statut le confirme. ⌘-clic l'enregistre de même, puis ouvre la copie enregistrée avec l'app par défaut, au premier plan.
- Les images affichées par le HTML du mail (`cid:`) ne sont pas des dalles. Une image marquée « intégrée » qu'aucun HTML n'affiche (photo d'Apple Mail dans un mail texte) en est une.
- Au clavier, `l` liste les liens du mail puis une ligne par fichier joint, avec son icône ; entrée enregistre le fichier, ⌘-entrée l'enregistre puis ouvre aussitôt la copie enregistrée, au premier plan.
- `l` cherche aussi les **expéditeurs du fil** dans le carnet d'adresses du Mac (les auteurs des messages reçus, pas les personnes en copie) : pour chaque fiche trouvée, une ligne par numéro de téléphone (« Alice Martin — mobile », entrée appelle) puis « Fiche de Alice Martin », qui ouvre Contacts. Un numéro déjà présent dans le mail n'apparaît qu'une fois, sous le nom. L'accès est demandé par macOS au premier `l` ; refusé, la barre de statut le dit une fois. Rien du carnet n'est copié dans la base.
- En HTML, les mêmes dalles suivent le corps du mail.

### Expéditeur

- Une seule ligne par message : « nom · date · adresse ». Message déplié : l'adresse complète (« ▾ Alice Martin · 30 oct. 14:00 · alice@atelier-dupuis.fr ») ; les destinataires, quand ils valent d'être montrés (À, Cc), sur la ligne suivante. Message replié : le domaine seul, puis l'aperçu (« ▸ Alice Martin · 30 oct. 11:00 · @atelier-dupuis.fr  Peux-tu… ») ; sans nom, l'adresse tient lieu de nom et n'est pas répétée. En texte comme en HTML.
- La flèche (▸ replié, ▾ déplié) précède le nom dans une conversation seulement : un clic sur la ligne replie ou déplie le message. Un mail seul n'a ni flèche ni repli : il est toujours déplié.
- Alerte : l'adresse passe dans la couleur d'alerte quand le mail a échoué à l'authentification, ou quand le nom affiché montre une autre adresse ou un autre domaine que le vrai (« service@paypal.com » <x7@promo-mail.ru>, « PayPal.com »). Un domaine du nom dont le vrai domaine se termine par lui ne compte pas (« Le Monde.fr » depuis news.lemonde.fr).
- Un clic sur l'adresse la copie (la barre de statut le dit) ; ⌘-clic filtre la liste sur cet expéditeur (pastille « de : »).
- Les destinataires gardent leur nom seul.
- Carnet d'adresses : au bout de la ligne d'un message déplié, après l'adresse, une icône dit si l'expéditeur est dans le carnet d'adresses du Mac (Contacts) : un carnet s'il y est, une silhouette avec un point d'interrogation sinon. Aucune icône pour les messages de l'utilisateur, ni tant que le carnet n'a pas pu être lu (pas d'accès) : mieux vaut rien qu'un « absent » faux. Un clic sur l'icône fait comme `ctrl-c`.
- `ctrl-c` : sur l'expéditeur du message courant, ouvre sa fiche dans Contacts s'il y est, sinon copie son adresse. `ctrl-C` (avec Maj) : liste les mails de cet expéditeur, tous dossiers confondus, comme Entrée sur l'écran des contacts.

### Images distantes

Charger une image distante apprend à l'expéditeur que le mail a été ouvert, quand, et depuis quelle adresse IP. Les pixels de suivi n'ont pas d'autre usage. Règles :

- **Pixels de suivi retirés par le noyau** à la réception : une image distante de 2 pixels ou moins (`width`, `height` ou style), ou masquée par son style (`display: none`, `visibility: hidden`, `opacity: 0`), perd sa source et n'est jamais demandée, quel que soit l'affichage. Leur nombre s'affiche sur la ligne de titre : un œil barré suivi du nombre (l'infobulle dit « pixels bloqués : N »). La ligne de titre (sujet, désabonnement, pixels, text | html | images) passe sur deux lignes, le sujet seul puis les contrôles à droite, dès que le sujet entier ne tient plus à côté des contrôles. La détection se fait avant toute requête : une fois la requête partie, l'ouverture est signalée.
- **Autres images distantes** : leur adresse est conservée sous une forme que la vue web ne charge pas seule (`nm-remote:`). Le mot `images` du sélecteur n'apparaît que si la conversation en contient.
- **Mode `images`** (clic ou `vi`) : l'app télécharge chaque image une fois, en HTTPS uniquement (`http:` est converti, jamais envoyé en clair), sans cookies, sans `Referer`, avec un User-Agent neutre, jamais vers la machine ni le réseau local, 10 Mo au plus et seulement du contenu `image/…`. Les images sont gardées en cache : rouvrir un mail ne refait aucune requête. Seuls les messages **dépliés** chargent leurs images.
- **Mémorisé par expéditeur, appliqué aux seuls mails authentifiés** (`pass`, `authentification-expediteur.md`) : sans authentification, n'importe qui peut écrire au nom d'un expéditeur de confiance. Pour un autre mail (import compris), la conversation s'ouvre en HTML ; choisir `images` charge les images **pour cette conversation seulement**.
- **Jamais dans Junk, Quarantine et Banned**, même sur demande.
- Une barrière de plus dans la vue web : une liste de blocage WebKit refuse toute ressource `http(s)`, au cas où une source distante passerait l'assainissement.

Limites assumées : l'adresse IP et l'heure de la première ouverture restent visibles pour chaque image chargée ; un mail dont chaque image porte un jeton propre au destinataire signale l'ouverture dès qu'on charge ses images ; un pixel masqué par un élément parent, ou sans aucune taille déclarée, n'est pas reconnu. Le préchargement de toutes les images à la réception (à la manière d'Apple Mail) est écarté : il confirmerait l'adresse à tous les expéditeurs, spammeurs compris.

### Désabonnement

À gauche du sélecteur `text | html | images` de la ligne de titre, un bouton de désabonnement apparaît quand le dernier mail reçu de la conversation donne un moyen de se désabonner. Il est masqué dans Junk, Quarantine et Banned : se désabonner d'un spam confirme au spammeur que l'adresse est lue (le noyau refuse aussi).

- **Identification, par le noyau à la réception** : d'abord l'en-tête `List-Unsubscribe` (RFC 2369), fiable ; à défaut, un lien du corps dont le texte annonce un désabonnement (plusieurs langues). Le bouton distingue les deux : lien de l'en-tête, « propre », ou lien du corps, qui ouvre une page de l'expéditeur dans le navigateur.
- **Action** : avec `List-Unsubscribe-Post: List-Unsubscribe=One-Click` (RFC 8058), le noyau envoie lui-même la requête et la barre de statut confirme. Sinon la page s'ouvre dans le navigateur ; une adresse `mailto:` seule s'ouvre dans Mail en attendant l'envoi SMTP. Un échec du désabonnement en un clic ouvre la page.
- **Ensuite** : l'expéditeur est noté « désabonné le … » (affiché à la place du bouton, et journalisé), sans changer sa décision ni ses mails.
- **Clavier** : ⌘U, comme dans Superhuman ; le modificateur évite de déclencher par erreur une action irréversible (`u` seul bascule lu / non lu).

### Lecture

- Un mail choisi par l'utilisateur (`j` `k`, un clic, le mail suivant après un déplacement) est marqué lu après une seconde d'affichage. Parcourir vite la liste ne marque rien. En une colonne, la liste seule ne montre pas le contenu : rester sur une ligne ne marque rien. Ouvrir le mail (Entrée, double-clic) le marque lu aussitôt ; dans le mail ouvert, j / k suivent la règle de la seconde.
- Un mail sélectionné par l'app (le premier à l'ouverture d'une liste, ou celui qui remplace un mail sorti de la liste) n'est pas marqué lu tout seul. Il l'est quand l'utilisateur le quitte pour un autre mail de la liste après au moins une seconde. Le déplacer ou changer de dossier ne le marque pas.
- Dans une liste filtrée (non lus, avec drapeau…), le mail sélectionné qui ne correspond plus (lu après une seconde, `u`, `U`, `*`) reste sous le curseur, affiché avec son nouvel état, jusqu'à ce qu'on le quitte ; il sort alors de la liste. Sans cela, une liste de mails non lus se vidait toute seule, un mail par seconde.
- Consulter un mail de Quarantine ne le marque **jamais** lu, quelle que soit la règle ci-dessus et où qu'il soit listé (Everything compris) : on juge un expéditeur inconnu, on ne lit pas son mail, et le point non lu continue de dire quels mails attendent une décision. Règle de l'app : le noyau accepte toujours `set_thread_seen`, le CLI peut donc marquer lu sur demande explicite.

### Quarantine

En Quarantine, la seule chose à faire d'un mail est de le déplacer vers une cible (`dossiers.md`, « Cibles proposées »). Les cibles deviennent donc des touches directes, sans le préfixe `m` : on économise une touche sur le geste le plus répété du dossier.

- **Barre de statut** : les blocs `m` et `s` disparaissent, le bloc IA reste, seul après `v` (trier le dossier est son premier prompt) ; à l'extrémité droite de la barre, juste avant `?`, la flèche de déplacement puis chaque cible suivie de sa touche : `Inbox i  Newsletter n  Paperwork p  Junk x  Banned b  Deleted d`. Alignées à droite, elles restent au même endroit quels que soient le compte, le dossier et les pastilles à gauche ; un message (résultat d'un déplacement) s'affiche avant elles et c'est lui qui est coupé si la place manque. Un clic sur une cible déplace comme sa touche.
- **Une touche déplace** : `i` fait ce que fait `mi` ailleurs, pour les mails marqués ou le mail sous le curseur ; `z` annule. `d` déplace dans **Deleted** (Archive n'est pas une cible depuis la Quarantine), alors qu'il archive partout ailleurs. Choix assumé : la règle « la lettre de la cible » prime sur le sens habituel de `d` ; Deleted se rattrape par `z`.
- **`m`** n'ouvre pas de menu : la barre de statut dit quelles touches taper.
- **Refusées** : `u` `U` (lu / non lu), `t` (tag), `g` (groupe), `r` (répondre : écrire à l'expéditeur l'accepterait, « Écrire à quelqu'un vaut décision Inbox »), `s` (enregistrer : son bloc quitte la barre ; un mail en Quarantine est à trier, pas à conserver). La barre de statut dit qu'en Quarantine un mail se déplace ou reçoit un drapeau, rien d'autre.
- **Restent** : `*` (drapeau), la navigation et les marques, `v`, `l`, `z`, Suppr.
- **Personne ou robot** : dans la colonne de l'expéditeur, une icône précède le nom : une silhouette (couleur d'accent) quand une personne a écrit le mail nommé, un engrenage (atténué) quand c'est un robot (`expediteurs.md`). Parmi les robots de la Quarantine, une personne est le mail qui mérite le plus probablement l'Inbox. C'est le verdict de ce mail-là (`message.sender_kind`, champ `correspondent_kind` de la ligne), pas celui de l'adresse : une newsletter envoyée depuis l'adresse d'une personne reste un robot. Sans verdict (pas d'expéditeur lisible, rattrapage pas fait), la place reste vide pour que les noms restent alignés ; l'icône prend sa place sur le nom, jamais sur les autres colonnes. Forcer la provenance d'une adresse (écran Contacts, `o`) recharge la liste et l'icône suit. Ailleurs, pas d'icône : le filtre humain / robot du sélecteur d'attributs fait ce travail et le nom garde sa place.

Cela vaut quand un dossier Quarantine est listé (d'un compte, ou de « all »). Dans Everything ou une liste de tag, groupe ou contact, les lignes viennent de plusieurs dossiers : les blocs `m` et `s` et les touches habituelles restent.

### Sélecteur d'attributs

Le sélecteur est aussi haut que toutes ses lignes, « effacer » compris, quelle que soit la taille du texte : ses lignes sont peu nombreuses et fixes, et une ligne qu'il fallait faire défiler pour voir passait inaperçue (avant novembre 2026, Date mail… et « effacer » étaient sous le bord). Seule une fenêtre trop basse le réduit ; ses lignes défilent alors.

À droite du compte et du dossier, dans la barre de statut, une loupe (touche `f`, ou un clic ; `f` et `F` échangés en novembre 2026 : la recherche par attributs, d'usage constant, prend la minuscule, le transfert `F` la majuscule) ouvre le sélecteur d'attributs, qui affine la liste courante (dossier d'un compte, ou de « all », ou mails d'un tag, d'un groupe ou d'un contact, dont la barre de statut garde alors « tag : … » suivi de la loupe et des pastilles) ; les attributs se combinent par « et ».

Ses lignes vont par couples, un trait fin et discret entre deux couples, chaque ligne commençant par une majuscule (novembre 2026) : Non lus / Lus ; Drapeau / Sans drapeau ; De… / À… ; Sujet… / Texte… ; Date conversation… / Date mail… ; Groupe… / Tag… ; Humain / Robot ; Pièce jointe / Sans pièce jointe ; Avec ma réponse / Sans ma réponse ; Conversations / Mails seuls ; puis, après un trait, « Retirer tous les attributs » quand un attribut est actif. Un couple réunit un attribut et son contraire, ou deux attributs de même nature : l'utilisateur voit d'un coup d'œil la logique du menu. Dès qu'on tape, les lignes sont classées par pertinence et les traits disparaissent. Les pastilles gardent la minuscule (« non lus », « tag : Urgent »). Avant, les attributs oui / non venaient d'abord, puis ceux à valeur, en un seul bloc. « Répondus » est devenu « Avec ma réponse » : c'est une réponse de l'utilisateur qui compte ; « répondus » reste un mot caché qui trouve la ligne.

- non lus / lus, drapeau / sans drapeau, pièce jointe / sans pièce jointe, avec ma réponse / sans ma réponse (répondu par l'utilisateur : un mail reçu de la conversation porte `\Answered`, ou elle contient une réponse de l'utilisateur dans Sent, un mail de Sent avec `In-Reply-To`, quel que soit le logiciel qui l'a envoyée ; un mail que l'utilisateur a écrit seul n'est pas « répondu ». Avant (novembre 2026), seul le drapeau comptait : les réponses envoyées par un logiciel qui ne le pose pas, et celles de NiceMailer, qui ne le posait pas, étaient ignorées), conversations / mails seuls ;
- De…, À… (À et Cc ; une partie d'un nom ou d'une adresse, contacts suggérés pendant la frappe ; plusieurs valeurs combinées par « ou » ; le nom est celui du contact, `address.name`, et non le nom d'affichage de chaque mail : un mail envoyé par une adresse partagée, comme `notifications@github.com` au nom de Thomas, n'est pas « de : thomas »), Texte… (mots du sujet et du corps), Sujet… (mots du sujet seul), Date conversation… (aujourd'hui, 7 derniers jours, 30 derniers jours, cette année, plus d'un an, ou après / avant une date tapée, ou depuis / il y a plus de une durée tapée), Date mail… (les mêmes choix) ;
- Groupe… (ajouté en novembre 2026) : les groupes de contacts existants (`groupes.md`), proposés dans le panneau ; une conversation en est quand un de ses mails des dossiers listés vient d'un membre ou lui est adressé (À, Cc). Plusieurs groupes se combinent par « ou ». Le panneau liste les groupes choisis d'abord (`•` voulu, `−` exclu), puis les autres du plus pertinent au moins pertinent (« Groupes dans la colonne des dossiers »), chacun avec son nombre de membres ; la frappe filtre par nom. Entrée choisit ou retire le groupe surligné et **le panneau reste ouvert** pour en choisir un autre, la liste derrière suivant chaque choix ; échap le ferme. C'est la seule exception à « entrée choisit et ferme » : choisir plusieurs groupes est l'usage visé, et rouvrir le panneau pour chacun coûterait deux touches par groupe. Pastilles « groupe : VIP », « hors groupe : VIP ». Sans groupe, le panneau dit que `g` en crée un. La dernière ligne, « sans groupe » (ajoutée en novembre 2026), garde les conversations dont aucun mail des dossiers listés n'est d'un membre d'un groupe ni ne lui est adressé : la règle d'un groupe exclu, pour tous les groupes. Choisie, elle passe en tête marquée `•`, avec la pastille « sans groupe » ; elle et un groupe voulu se remplacent l'un l'autre (ensemble, la liste serait vide), les groupes exclus restent. Elle est en dernier pour qu'entrée sur la première ligne prenne toujours le groupe le plus pertinent ; « aucun » la trouve.
- Tag… (ajouté en novembre 2026) : comme Groupe…, une liste où choisir : les tags existants, une ligne par nom (les tags de même nom de tous les comptes n'en font qu'un, comme pour `;T`), les choisis d'abord (`•` voulu, `−` exclu), puis les plus employés, chacun avec son nombre de mails ; la frappe filtre par nom, `-nom` exclut. Une conversation en est quand un de ses mails porte un des tags choisis (« ou ») ; un tag exclu l'écarte dès qu'un de ses mails le porte. Entrée choisit ou retire et le panneau reste ouvert, comme pour Groupe…. Pastilles « tag : Urgent », « hors tag : Urgent ». Sans tag, le panneau dit que `t` en crée un. La dernière ligne, « sans tag », garde les conversations dont aucun mail ne porte de tag, sur le modèle de « sans groupe ». Le noyau reçoit des noms (`tags`, `not_tags` de `list_threads`), et `no_tag`, `no_group`.

**Deux critères de date** (ajouté en novembre 2026). **Date conversation…** porte sur la date de la conversation, celle de son dernier mail (sans ceux de Deleted, Junk et Banned, comme la date affichée dans la liste) : « après le 2020-01-01 » garde les conversations encore actives depuis 2020, « avant le 2020-01-01 » celles qui se sont tues avant. C'est le critère de presque toutes les recherches, et il ne coûte rien : la date est celle sur laquelle la liste est triée. **Date mail…** porte sur chaque mail : une conversation qui court de 2016 à 2026 est trouvée par « avant le 2020-01-01 », parce que certains de ses mails sont de 2016. Elle reste triée et datée par son dernier mail. Ce critère regarde les mails des dossiers listés, comme De… (« Négation ») ; il est plus lourd (son coût suit le nombre de mails de l'intervalle), d'où un critère à part plutôt qu'un changement du premier. Chaque critère a ses deux bornes et ses pastilles ; celles de Date mail… sont préfixées (« mail : 7 derniers jours »). Les deux se combinent.

La ligne de saisie de chaque panneau de valeur dit ce qu'on peut taper tant que rien n'est tapé : les panneaux de date (Date conversation… et Date mail…, les mêmes ; « Date… » ci-dessous vaut pour les deux) s'ouvrent sur leurs préréglages, et une aide montrée seulement quand le panneau est vide restait invisible. Date… accepte `2026-10-01`, une date à barres ou à points dans l'ordre de la région du Mac (`1/10/2026` ou `1/10/26` en France, `10/1/2026` aux États-Unis, `2026/10/1` au Japon : `localisation.md`, section 3.4), et **six chiffres**, lus d'abord dans l'ordre de la région (`jjmmaa` en France, `mmjjaa` aux États-Unis, `aammjj` au Japon), puis en `aammjj` (en `jjmmaa` pour une région qui met l'année d'abord). Chaque lecture qui donne un vrai jour propose « après le » et « avant le » ce jour : une seule lecture valide donne deux lignes (`311299` : 31 décembre 1999), deux lectures valides en donnent quatre, et chaque ligne dit à droite sa lecture (« jj mm aa », « mm jj aa », « aa mm jj ») : en France, `010120` est le 1er janvier 2020 ou le 20 janvier 2001. L'aide du panneau donne ses exemples dans cet ordre. Deux lectures qui tombent le même jour comptent une fois (`111111`). Une année sur deux chiffres sous 70 est 20aa, à partir de 70 19aa (pas de mail avant 1970).

Date… accepte aussi une **durée** : un nombre et une unité, par sa première lettre en français ou en anglais, ou son mot entier : `j`/`d` jour, `s`/`w` semaine, `m` mois, `a`/`y` année (`10j`, `3 s`, `8w`, `6m`, `2 ans`). Les deux langues sont toujours comprises, les lettres ne se chevauchant pas. Sans nombre, c'est 1 (`m` : un mois). Le panneau propose « depuis 10 jours » et « il y a plus de 10 jours », avec à droite la date obtenue. Toutes les unités se comptent depuis maintenant selon le calendrier : `1j` est hier à la même heure, `1m` le même jour du mois précédent, donc `7j` et `1s` coïncident.

Les deux bornes de date sont indépendantes et forment un intervalle : chaque ligne fixe la borne basse (« après », « depuis », les préréglages sauf le dernier) ou la borne haute (« avant », « il y a plus de », « plus d'un an ») et garde l'autre. « après le 2020-01-01 » puis « avant le 2020-07-01 » donne le premier semestre 2020, en deux pastilles supprimables séparément. Rechoisir une borne la remplace. Une ligne qui ne laisserait aucun jour entre les deux bornes l'indique à droite (« rien entre les deux ») sans être refusée.

Texte… ne cherche pas dans les adresses : chercher « dupont » dans le texte ne doit pas ramener tous les mails de Dupont, c'est le rôle de De… et À…. Seule la recherche `/` (« Chercher partout ») reste le joker qui cherche aussi dans l'expéditeur et les destinataires.

#### Accès au clavier

Le sélecteur s'ouvre sur sa première ligne et entrée choisit la ligne en surbrillance : ce qui compte est la ligne qui arrive en tête quand on tape. Les lignes sont classées, sans tenir compte de la casse ni des accents, et à rang égal gardent l'ordre de la liste :

1. le libellé commence par la frappe (`lu` → « lus », avant « non lus » qui ne fait que le contenir) ;
2. chaque mot tapé commence un mot du libellé, dans n'importe quel ordre (`jointe sans` → « sans pièce jointe ») ;
3. les lettres tapées sont les initiales des mots du libellé, pour un seul mot tapé d'au moins deux lettres (`nl` → « non lus », `sd` → « sans drapeau », `spj` → « sans pièce jointe », `sm` → « sans ma réponse ») ;
4. chaque mot tapé apparaît n'importe où dans le libellé.

Les attributs à valeur ont aussi des mots de recherche cachés, traduits : `exp`, `expéditeur` trouvent De… ; `cc`, `copie`, `destinataire` trouvent À… ; `texte`, `mots`, `corps` trouvent Texte… ; `titre`, `objet` trouvent Sujet… ; `quand`, `jour`, `période` trouvent Date conversation… ; `message` trouve Date mail…. Les libellés français sont choisis pour que deux lettres suffisent toujours : `de` De…, `da` Date conversation… (le critère courant vient avant Date mail…), `dm` Date mail…, `dc` Date conversation…, `dr` drapeau, `a` À…, `l` lus, `no` non lus, `pi` ou `pj` pièce jointe, `av` avec ma réponse, `sm` sans ma réponse, `co` conversations, `m` mails seuls, `te` Texte…, `su` Sujet…. « À… » remplace « Destinataire… », qui commençait comme « De… », et reprend l'en-tête « À : » du mail et la pastille « à : ». Ce classement vaut pour tous les sélecteurs (`@`, `l`, `t`, `g`, `f`).

#### Mots avec ponctuation

Dans `/`, Texte… et Sujet…, chaque mot tapé est cherché comme début de mot, sans tenir compte des accents ni de la casse. Un mot qui contient un signe de ponctuation (`.` `@` `/` `:` `_`…) est en plus cherché **tel qu'il est écrit** : `paris.fr` trouve « paris.fr », « contact@paris.fr », « mairie-paris.fr », mais plus « 75016 PARIS / France » ni « Paris FR » (ajouté en novembre 2026). L'index plein texte découpe les mots sur la ponctuation et lisait `paris.fr` comme « paris » suivi d'un mot commençant par « fr » : sur une vraie boîte, 14 542 mails au lieu des 417 qui contiennent « paris.fr ». Le tiret et l'apostrophe ne comptent pas : ils lient les mots de la prose (`rendez-vous` trouve « rendez vous », `l'eau` trouve « l’eau »). Les accents ne sont pas ignorés dans la partie écrite telle quelle (`eleve.fr` ne trouve pas « élève.fr »). Ce contrôle lit le texte des mails candidats : environ 0,4 s par page sur 400 000 mails, pour ces mots seulement. Un mot avec ponctuation précédé d'un tiret est exclu de la même façon.

#### Négation

L'attribut **humain / robot** (ajouté en novembre 2026, `expediteurs.md`) garde les conversations écrites par une personne, ou celles où personne n'a écrit ; il forme un couple, Humain / Robot, après Groupe… / Tag…. Les mots cachés `personne`, `gens` trouvent « humain » ; `automate`, `machine`, `masse` trouvent « robot ».

Chaque attribut oui / non a son contraire en ligne propre (« sans drapeau », « non lus », « sans ma réponse »…), atteint par ses initiales ; choisir le contraire d'une valeur active la remplace.

Pour De…, À…, Groupe…, Texte… et Sujet…, la négation se choisit dans le panneau de la valeur (pour Groupe…, en tapant `-nom` : les lignes deviennent « pas VIP »), sans doubler les lignes du sélecteur :

- la frappe propose deux lignes : `+ acme` (entrée) et `− pas acme` (flèche bas puis entrée) ; taper `-acme` (tiret devant) ne laisse que l'exclusion, et les contacts suggérés sont alors proposés en exclusion ;
- une valeur est voulue ou exclue, pas les deux : le dernier choix l'emporte ;
- la pastille le dit : « de : acme », « pas de : acme », « à : acme », « pas à : acme » ; Texte… et Sujet… gardent le tiret des mots exclus (« texte : renouvellement -facture ») ;
- les valeurs positives d'un attribut se combinent par « ou », ses valeurs exclues par « et » : « de : alice », « de : bob », « pas de : @acme.fr » garde les mails d'Alice ou de Bob qui ne viennent pas d'acme.fr. Une conversation est exclue dès qu'un de ses mails des dossiers listés correspond à une valeur exclue. Les dossiers listés, et pas seulement celui où la conversation s'affiche : dans Everything, une conversation de l'Inbox s'affiche dans l'Inbox, et « pas de : @acme.fr » doit voir le mail d'acme.fr qu'elle a encore dans Archive. La règle vaut pour De…, À…, pièce jointe et Date mail… ;
- **domaine** : une valeur qui commence par `@` désigne un domaine exact et ses sous-domaines (`@acme.fr` couvre `jean@acme.fr` et `x@mail.acme.fr`, pas `x@notacme.fr`). Sans `@`, la valeur reste une partie de nom ou d'adresse ;
- dans Texte… et Sujet…, `-mot` exclut un mot parmi d'autres (`renouvellement -facture`). Choisir de nouveau Texte… ou Sujet… ouvre le panneau sur les mots actuels, sélectionnés : taper (ou coller) les remplace, entrée seule les garde, une flèche lève la sélection pour les éditer (ajouter une exclusion décidée après coup : flèche droite, puis ` -facture`), effacer puis entrée retire le critère. Auparavant les nouveaux mots s'ajoutaient aux anciens, invisibles dans le panneau : une seconde recherche par sujet restreignait la première au lieu de la remplacer ;
- `/` accepte aussi `-mot`, cherché partout, adresses comprises.

Exemple, la gestion des registrars : trouver les mails qui parlent du domaine acme.fr sans ceux envoyés ou reçus par ce domaine, c'est Texte… `acme.fr`, puis De… `-@acme.fr`, puis À… `-@acme.fr` ; trois pastilles « texte : acme.fr », « pas de : @acme.fr », « pas à : @acme.fr ».

Les attributs avec « … » agrandissent aussitôt le panneau pour choisir la valeur. Choisir de nouveau un attribut actif le retire. Les attributs actifs s'affichent en pastilles dans la barre de statut ; un clic sur une pastille la retire, échap les retire toutes ; ils restent quand on change de dossier ou de compte.

`/` est le raccourci de la recherche partout : il ouvre un panneau intitulé « Chercher partout » ; entrée va dans **All Mail** de la vue (tous les dossiers sauf Deleted, Junk et Banned, tous les comptes dans « all ») avec ces mots ; tapé depuis Everything, il reste dans Everything. Le résultat est la liste ordinaire, une ligne par conversation avec son dossier, la pastille et `f` pour affiner. Échap retire les attributs et revient au dossier d'où `/` a été tapé ; si l'on est allé ailleurs entre-temps (`;`, `@`, clic sur un dossier), échap retire seulement les attributs. Un second `/` remplace les mots et garde le dossier de retour. `/` reste le joker le plus large : ses mots sont cherchés partout, sujet, expéditeur, destinataires et texte (pastille « partout : … »), là où Texte… laisse les adresses de côté.

### Groupes dans la colonne des dossiers

Ajouté en novembre 2026. Dans la présentation en trois colonnes (`v3`), la colonne des dossiers liste sous les dossiers, après un titre « Groupes », les groupes de contacts (`groupes.md`). Un clic sur un groupe **filtre la vue courante** (dossier, dossier de « all », Everything, liste d'un tag…) sur les mails de ses membres : c'est l'attribut Groupe…, avec sa pastille dans la barre de statut, et non une liste à part comme `;g` puis entrée ; un second clic le retire. Plusieurs groupes cliqués se combinent par « ou ». ⌘-clic exclut le groupe. Un groupe actif est marqué `•` (exclu : `−`, barré) dans la couleur d'accent, les autres `@`. Sa ligne a aussi le fond gris du dossier courant, qu'il ait été choisi d'un clic ou dans le panneau Groupe… (`f`), voulu ou exclu : le dossier et les groupes actifs disent ensemble ce que montre la liste. Comme tout attribut, les groupes restent quand on change de dossier, et échap les retire. Sans groupe, ni titre ni bloc.

La colonne montre **huit groupes au plus**. Au-delà, elle garde les plus pertinents et finit par « N autres… », qui ouvre le panneau Groupe… avec tous les groupes (le titre « Groupes » l'ouvre aussi). La pertinence croise deux signaux donnés par le noyau (`list_groups`) :

- la **date de dernière modification** du groupe ou de ses membres (ajout, retrait) : un groupe qu'on vient de remplir est celui sur lequel on travaille ;
- le **nombre de mails reçus de ses membres** (somme des `address.sender_count`, compteur tenu par l'ingestion : rien n'est compté à la demande).

Les deux classements sont **entrelacés** : le groupe le plus récent, puis le plus gros pas encore pris, puis le suivant le plus récent, et ainsi de suite ; à égalité, l'ordre d'affichage. Un score qui mélangerait une date et un nombre demanderait des poids sans valeur naturelle ; l'entrelacement n'en demande pas, se prévoit facilement, montre tout de suite un groupe neuf et garde un gros groupe ancien. Les groupes actifs sont toujours montrés, même au-delà de huit : un filtre invisible ne pourrait pas se retirer d'un clic. Les groupes montrés restent dans l'ordre d'affichage (celui de leur création, `position`) : un groupe ne change pas de ligne quand un mail arrive, il entre ou sort seulement.

### Dossiers virtuels

Trois dossiers virtuels réunissent des dossiers de la vue (de tous les comptes dans « all »). Chacun termine le bloc de ce qu'il réunit dans le menu `;` et la colonne des dossiers, et s'y distingue des vrais dossiers : une icône de pile devant son nom, et la couleur d'accent (la police à chasse fixe du thème n'a pas d'italique). Réorganisé en novembre 2026 : Work ajouté, All Mail passé de `E` à `m`.

- **Work** (`;o`) : Inbox, Todo et Waiting ensemble, ce qui reste à traiter. Chaque ligne affiche son dossier. Ses critères (De…, À…, pièce jointe, Date mail…) ne regardent que les mails de ces trois dossiers.
- **All Mail** (`;m`) : tous les dossiers sauf Deleted, Junk et Banned, Quarantine comprise. C'est ce que veulent la plupart des recherches : ce qu'on a supprimé ou rejeté n'y encombre pas les résultats, et le mail d'un expéditeur pas encore accepté s'y trouve. Le nom est celui de Gmail, pour le même sens (tout sauf la corbeille et le spam). Une conversation dont le dernier mail a été supprimé y garde sa place et sa date, celles de son dernier mail resté dans All Mail ; ses critères (De…, À…, pièce jointe, Date mail…) ne regardent que les mails de ces dossiers. Ce sont les conversations dont le dossier n'est ni Deleted, ni Junk, ni Banned (`dossiers.md`, « Dossier d'une conversation ») : une conversation rejetée n'y revient pas par la réponse de l'utilisateur restée dans Sent.
- **Everything** (`;e`) : vraiment tous les dossiers.

Une conversation présente dans plusieurs dossiers (Inbox et Sent) n'y figure qu'une fois, dans son dossier (`dossiers.md`, « Dossier d'une conversation ») : l'Inbox, et non Sent, quand le dernier mail est la réponse de l'utilisateur. Comme la liste mélange tous les dossiers, chaque ligne affiche son dossier (colonne discrète avant le nombre de messages), et le lecteur l'affiche à l'extrémité droite de la ligne du sujet. Dans « all », le compte suit le dossier (« Archive · perso ») : toujours dans le lecteur, et dans la liste seulement si le sujet garde au moins 30 caractères ; sinon le dossier seul.

Il y a deux contextes totalement différents dans l'application : 
- "navigation" : la gestion du stock de mail d'une part. lecture, recherche, screening, déplacement, etc
- "edition" : L'écriture d'un nouveau mail (from scratch ou en réponse) d'autre part. 

Les raccourcis clavier sont différents en fonction de ces deux contextes. 

Voici la liste des principaux raccourcis clavier dans le contexte "navigation"


hjkl : naviguer dans les mails
j et k : mail suivant et précédent
ctrl-d et ctrl-u : une demi-page plus bas ou plus haut dans la liste des mails, comme dans Zed ; le curseur avance de la moitié des lignes visibles et s'arrête au premier ou au dernier mail
h et l : change de "colonne"
	Si on est focus sur la liste des mails, h déclenche l'écran pour passer à un autre dossier. Dans l'interface classique, c'est la barre à gauche. Dans l'interface geek, ce rôle est tenu par `;` : `h` n'y fait rien. 
	Et depuis la liste des mails, l déclenche l'écran de "link".

Avec une majuscule (JK), ou maj + flèche bas / haut, on passe au mail suivant et précédent en conservant le mail actuel sélectionné. On peut en effet faire des sélections multiples pour déplacer ou taguer plusieurs mails en même temps.  

À la souris (thème geek) : un clic sélectionne un mail et efface les marques ; ⌘-clic marque ou démarque un mail ; ⇧-clic marque tous les mails entre le mail courant et le mail cliqué. Dans les écrans contacts, tags et groupes, un double-clic équivaut à entrée. Un clic agit immédiatement, sans attendre un éventuel double-clic.


Les opérations classiques sur un mail sont (elles s'appliquent aux mails marqués avec J/K, sinon au mail sélectionné) : 
r répondre à tous (`reponse.md`) : contexte édition, ⌘↩ envoie, ⇧⌘↩ choisit où ranger la conversation puis envoie, échap garde le brouillon, ⌘⎋ l'abandonne, ⇧⌘T bascule texte seul / mis en forme, ⌃I ouvre le menu IA ; ⇥ et ⇧⇥ passent de champ en champ (À, Cc, Cci, Objet, texte), ⌥⌘B ouvre la ligne Cci, ⌥⌘F choisit l'adresse (donc le compte) d'envoi ; le texte est un champ standard du Mac, avec ses raccourcis (`edition.md`)
R répondre à l'expéditeur seul (`reponse.md`, section 4 bis)
F forwarder : transférer le mail et ses fichiers
c écrire un nouveau mail
d mark done (archiver)
m move (déplacer), puis choisir le dossier. mt déplacer dans le dossier Todo, md déplacer dans le dossier Deleted. Le menu ne propose que les cibles permises depuis le dossier du mail. En Quarantine, la lettre seule suffit (`i`, `n`, `p`, `x`, `b`, `d`) : voir « Quarantine ».
del supprimer (déplacer dans Deleted)
t tag (ajouter ou retirer un tag : « Ajouter et retirer un tag ou un groupe »)
g group (ajoute l'expéditeur dans un groupe, ou l'en retire : « Ajouter et retirer un tag ou un groupe »)
l link (panneau des liens du mail ; entrée ouvre le lien dans le navigateur). Dans un mail de code (code de vérification à usage unique envoyé par un site ou une application : « Votre code de vérification : 610731 »), la toute première ligne est le **code de vérification**, avec une icône de cadenas : entrée (ou ⌘-entrée) le copie dans le presse-papiers, la barre d'état le confirme. Dans une conversation, c'est le code du dernier message qui en donne un. Le noyau reconnaît le code (règles dans `docs/core-library.md`, « Verification codes ») ; une conversation entre personnes (réponse, transfert) n'est pas un mail de code. Ensuite, « Lien vers ce mail » : entrée copie un lien Markdown `[sujet](nicemailer://m/<id>)` vers le mail, pour le coller dans une note (Obsidian), ⌘-entrée l'URL seule ; un clic sur ce lien ouvre le mail dans NiceMailer. Un même lien (même destination, même derrière des adresses de suivi différentes) n'apparaît qu'une fois, avec son premier libellé suivi du nombre exact d'autres libellés qui y mènent : « ressent "les angoisses" du pays (et 2 autres) ». Des liens collés l'un à l'autre vers la même destination forment un seul lien, leurs textes réunis. D'abord les liens qui ont un texte (« Voir ma facture »), puis les numéros de téléphone trouvés dans le mail, avec une icône de téléphone (entrée lance l'appel dans l'app Téléphone du Mac), puis les pièces jointes, enfin les liens sans texte (le plus souvent une image menant à un réseau social). Le panneau est aussi large que la fenêtre le permet, pour lire les liens longs.
u lu / non lu
U marque lu, quel que soit l'état actuel (un mail lu reste lu, contrairement à u)
z annule le dernier déplacement ou la dernière décision (les mails reviennent, la décision précédente aussi, et le curseur revient sur la conversation rétablie) ; chaque z remonte d'une étape
\* drapeau
/ chercher partout : des mots n'importe où, adresses comprises, dans All Mail (voir le sélecteur d'attributs) ; échap revient au dossier de départ
? liste des touches, regroupées par usage (naviguer, lire, sélectionner, agir sur les mails, écrire, chercher et aller, fenêtre), une touche par ligne, dans un panneau centré de trois colonnes au plus, équilibrées en hauteur ; moins de colonnes si la fenêtre est étroite. Les commandes pas encore disponibles sont grisées. N'importe quelle touche ou un clic à côté la ferme
échap annule un préfixe, ferme un panneau, démarque, puis revient du tag, groupe ou contact listé au dossier
⌃⌘F met la fenêtre en plein écran et l'en sort, partout (liste, menu, réponse en cours d'écriture), que la barre de titre soit masquée ou non
en une colonne, sur la liste, espace ouvre le mail comme entrée ; mail affiché, espace et ctrl-f le font défiler d'une page vers le bas, maj-espace et ctrl-b vers le haut, en texte comme en HTML

Le prefix ";" permet de changer d’écran
; aller à un dossier. Le menu (et la colonne des dossiers) les range en quatre blocs, séparés par une ligne vide, chacun terminé par son dossier virtuel (« Dossiers virtuels ») : ce qui est à traiter, puis Work ; ce qui est classé, Quarantine comprise (son mail attend une décision sur l'expéditeur, pas un travail) ; ce qui a été écrit, puis All Mail ; ce qui a été jeté, puis Everything. Viennent ensuite les écrans Contacts, Tags, Groups, et History à part. Les dossiers propres à l'utilisateur forment un bloc après les dossiers classés. Le nombre de mails non lus de chaque dossier est aligné à l'extrémité droite de sa ligne.

i Inbox
t Todo
w Waiting
o Work

a Archive
n Newsletter
p Paperwork
q Quarantine

s Sent
r Drafts
m All Mail

d Deleted
x Junk
b Banned
e Everything

c Contacts
T Tags
g Groups

h History

Les lettres sont celles des dossiers (`dossiers.md`) : `j` est réservé à la navigation, d'où `x` pour Junk, y compris après un préfixe.

ainsi ;i va à l’inbox, ;p va à Paperwork, ;t va à Todo, etc

Le prefix ";" permet également d'aller aux autres écrans : 
c Contact (liste des adresses, avec leur provenance « humain » ou « robot », suivie de « * » quand l'utilisateur l'a choisie ; / filtre : entrée ferme la saisie et garde le filtre, qui reste aussi quand on quitte l'écran et qu'on y revient ; tant qu'il est actif, il est affiché en permanence dans la barre de statut, comme les pastilles des attributs (`/gazette ×`), et un clic dessus le retire ; m choisit la décision sur l'adresse, M sur son domaine : i Inbox, n Newsletter, p Paperwork, b Banned, - aucune ; o choisit la provenance : h humain, r robot, - automatique, voir `expediteurs.md` ; entrée liste les mails du contact, tous dossiers)
h History (affichage de l'historique issu de l audit log, plus récent d'abord)
T tag (majuscule : `;t` est Todo ; une ligne par nom, les tags de même nom des autres comptes compris, avec leur nombre de mails cumulé ; entrée liste les mails du tag, dans tous les comptes)
g group (entrée liste les mails des membres du groupe)

Sur ces écrans, j/k se déplacent et échap revient aux mails.

#### Écrans Tags et Groupes

Ajouté en novembre 2026. Sur les écrans `;T` et `;g`, deux touches agissent sur le tag ou le groupe sous le curseur ; la ligne d'aide de l'écran les rappelle.

- `r` renomme. Un panneau s'ouvre sur le nom actuel, sélectionné : la frappe le remplace, une flèche le garde pour le corriger. Entrée renomme ; échap annule. Le nom d'un autre tag ou groupe (sans tenir compte de la casse) est refusé, la ligne le dit : fusionner deux tags ou deux groupes n'est pas un renommage. Changer seulement la casse (« vip » en « VIP ») est permis.
- `d` supprime, après une question (`y` supprime, échap garde), qui donne le nombre de mails du tag ou de membres du groupe. La suppression ne s'annule pas avec `z`.

Un tag est renommé ou supprimé dans **tous les comptes** qui en ont un de ce nom, comme l'écran le présente en une ligne. Le renommage ne change que le nom affiché : le mot-clé IMAP enregistré sur chaque mail reste (`modele de donnees.md`, `tag`). Le changer réécrirait chaque mail tagué sur le serveur, pour un nom que seul NiceMailer affiche ; en contrepartie, un autre logiciel de mail (Thunderbird) continue d'afficher l'ancien mot-clé. La suppression, elle, est complète : le tag est retiré de tous ses mails, sur le serveur aussi, puis gardé comme pierre tombale ; recréé plus tard sous le même nom, il ne reprend aucun ancien mail. Un compte en lecture seule (`read_only`) n'est pas touché : son tag reste.

Un groupe renommé garde ses membres et sa place ; supprimé, il devient une pierre tombale (`groupes.md`, section 3), et aucun mail ne change.

Ce qui nomme le tag ou le groupe suit : les attributs (pastilles « tag : … », « groupe : … ») prennent le nouveau nom ou disparaissent, la liste des mails du tag ou du groupe (`;T` puis entrée) prend le nouveau nom ou revient au dossier, et l'en-tête du mail ouvert est relu.


Il y a deux autres préfixes : s et v. (`d` était prévu pour display, mais `d` seul archive : une touche ne peut pas être à la fois une action et un préfixe.)
s save (dans ~/Downloads ; un fichier existant n'est jamais écrasé ; s'applique aux mails marqués, ou à défaut au mail sous le curseur). Dans la conversation affichée, chaque ligne du menu porte sur le message sur lequel est la lecture (atteint par ctrl-k / ctrl-j ou un clic sur son en-tête, comme pour r), à défaut sur le dernier ; pour les autres conversations marquées, sur leur dernier message. On exporte ainsi toute une conversation, message par message. Les pièces jointes (a, g) sont celles du message choisi s'il y en a un, sinon de toute la conversation.
	c to clipboard (en première ligne) : copie dans le presse-papiers le texte d'un mail de chaque conversation (voir ci-dessus lequel), comme t (en-têtes From, To, Cc, Date, Subject, puis le corps), dans l'ordre de la liste, une ligne vide entre deux mails. Les paragraphes doivent rester exploitables une fois collés dans un document : un paragraphe tient sur une seule ligne, sans les retours à la ligne que l'app de l'expéditeur insère tous les 72 caractères (format=flowed, texte coupé à largeur fixe), et une seule ligne vide sépare deux paragraphes. Les lignes voulues restent telles quelles : citations (`>`, par profondeur), signature (après `-- `), listes, lignes indentées, tableaux, lignes courtes (formule de politesse, nom), suite de phrases une par ligne. La barre de statut le confirme : « Devis toiture » copié dans le presse-papiers ; pour plusieurs mails, leur nombre.
	a all attachments : les pièces jointes des mails marqués, ou à défaut du mail sous le curseur ; tous les messages de chaque conversation, ou seulement le message choisi dans la conversation affichée ; un fichier joint de nouveau à une réponse (même nom, type et taille) n'est enregistré qu'une fois
	g attachments but graphics (gif, png, ...) : de même, sans les images (logos, signatures, photos)
	p as pdf (P : enregistre puis ouvre) : un PDF A4 pour l'usage courant. Police Helvetica, l'objet en gras en titre ; toutes les parties, le nom tel qu'écrit dans ce mail et l'adresse (auteurs, Sender et Reply-To s'ils diffèrent de l'auteur, To, Cc, Bcc) ; date d'envoi (en-tête Date) et de réception par le serveur, à la seconde, avec le fuseau ; le corps entier ; la liste des pièces jointes (nom, type, taille). En pied de page, la seule pagination « Page n / N ». Le noyau lit tout dans le brut (`message_record`) ; l'app compose le PDF.
	e as eml file (un mail de chaque conversation, comme t et m)
	t as text file (T : enregistre puis ouvre le fichier au premier plan, dans l'app associée par macOS)
	m as markdown file (M : de même)
	Dans le menu, la ligne de t (et de m) rappelle à droite, en grisé, « T : et ouvrir ». Avec les flèches, ⌘-entrée sur la ligne fait de même. Au-delà de 5 mails, les fichiers sont enregistrés mais pas ouverts, et la barre de statut le dit.
	i as pdf with technical information (I : enregistre puis ouvre ; dernière ligne du menu) : le même PDF, destiné à être produit comme preuve, avec en plus le SHA-256 de chaque pièce jointe ; un relevé technique (Message-ID, References, en-tête Date tel qu'écrit, en-têtes de transit Received, Authentication-Results, DKIM-Signature…, taille et SHA-256 du message brut) ; et en pied de chaque page « Page n / N » et le SHA-256 du message brut. Ni le compte et le dossier, ni le verdict d'authentification de l'app, ni la date d'export n'y figurent. Sans brut, le PDF le dit : il ne peut pas être rapproché de l'original. La preuve elle-même reste le .eml (se), signé DKIM par le domaine expéditeur ; le PDF en donne l'empreinte.
v display (view)
	1 une seule colonne (masque la liste des dossiers et la lecture d'un mail). Entrée, ou un double-clic sur une ligne, affiche le mail dans toute la fenêtre et le marque lu aussitôt (j / k passent au mail suivant / précédent) ; Escape revient à la liste
	2 colonnes (presentation classique : liste des mails à gauche et lecture du mail à droite)
	3 colonnes (liste des dossiers à gauche, puis liste des mails au milieu et lecture du mail à droite)
	c creation date (mail trié par date de création, ou de réception)
	m modification date (mail trié par date de modification: permet de voir ce que l'on a modifié)
	h texte / HTML (choix mémorisé pour l'expéditeur)
	i images distantes (choix mémorisé pour l'expéditeur, appliqué aux seuls mails authentifiés)
	s barre de statut : en bas, en haut, masquée

Le nombre de colonnes, la position de la barre de statut et la largeur de la liste sont mémorisés.

Taille du texte, comme dans Zed : ⌘+ (ou ⌘=, sans Maj sur un clavier AZERTY) agrandit toute l'interface, ⌘− la réduit, ⌘0 revient à la taille par défaut, d'un point à la fois entre 9 et 24 points (12 par défaut). Une seule taille de base : polices, hauteur des lignes, largeur des colonnes, menus, sélecteurs et vue HTML en découlent. Elle est mémorisée.
