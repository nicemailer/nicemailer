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

### Thème geek : choix retenus

- Police monospace pour l'interface (SF Mono) ; corps des mails en **iA Writer Mono V**, livrée avec l'app (licence SIL OFL).
- Palette inspirée de Helix, en variante claire et sombre selon l'apparence de macOS.
- Corps des mails en texte par défaut. Un bouton `text | html` en haut du mail, et `vh`, basculent en HTML assaini ; le choix est mémorisé **par adresse d'expéditeur** (`address.body_format`, `schema.sql`).
- Barre de statut : le mode (`NOR`, ou le préfixe en attente, comme dans Helix) et l'emplacement (« compte · dossier », ou le tag, le groupe, le contact listé). Chaque élément s'ouvre d'une touche : le mode avec `?` (liste des touches), l'emplacement avec `h` (dossiers). Les messages (résultat d'une action, erreur) s'affichent à droite et disparaissent à la touche suivante.
- Les sélecteurs (`h`, `l`, `t`, `g`) sont des listes filtrables à la manière de Helix : on tape pour filtrer (chaque mot tapé doit apparaître), `ctrl-n` / `ctrl-p` ou les flèches pour se déplacer, entrée pour choisir, échap pour fermer. Pour `t` et `g`, taper un nom absent propose de le créer.

L'interface utilisateur de NiceMailer est, quel que soit le thème choisi, minimaliste et sobre. 

La fenêtre principale permet de lister les mails en deux colonnes : 
- A gauche, sur un petit tiers de l'écran, mais resizable, la liste des mails. 
- A droite, sur le reste de l'écran, l'affichage du mail sélectionné à gauche. 

Lors de l'ouverture d'une fenêtre, le premier mail est systématiquement sélectionné dans la liste, de sorte que la partie de droite affiche quelque chose. 

Il y a deux contextes totalement différents dans l'application : 
- "navigation" : la gestion du stock de mail d'une part. lecture, recherche, screening, déplacement, etc
- "edition" : L'écriture d'un nouveau mail (from scratch ou en réponse) d'autre part. 

Les raccourcis clavier sont différents en fonction de ces deux contextes. 

Voici la liste des principaux raccourcis clavier dans le contexte "navigation"


hjkl : naviguer dans les mails
j et k : mail suivant et précédent
h et l : change de "colonne"
	Si on est focus sur la liste des mails, h déclenche l'écran pour passer à un autre dossier. Dans l'interface classique, c'est la barre à gauche. Dans l'interface geek, c'est un panneau. 
	Et depuis la liste des mails, l déclenche l'écran de "link".

Avec une majuscule(JK) on passe au mail suivant et précédent en conservant le mail actuel sélectionné. On peut en effet faire des sélections multiples pour déplacer ou taguer plusieurs mails en même temps.  

À la souris (thème geek) : un clic sélectionne un mail et efface les marques ; ⌘-clic marque ou démarque un mail ; ⇧-clic marque tous les mails entre le mail courant et le mail cliqué. Dans les résultats de recherche et les écrans contacts, tags et groupes, un double-clic équivaut à entrée. Un clic agit immédiatement, sans attendre un éventuel double-clic.


Les opérations classiques sur un mail sont (elles s'appliquent aux mails marqués avec J/K, sinon au mail sélectionné) : 
r répondre (pas encore : contexte édition)
R répondre à tous (pas encore)
f forwarder (pas encore)
d mark done (archiver)
m move (déplacer), puis choisir le dossier. mt déplacer dans le dossier Todo, md déplacer dans le dossier Deleted. Le menu ne propose que les cibles permises depuis le dossier du mail.
del supprimer (déplacer dans Deleted)
t tag (ajouter un tag ; si tous les mails visés l'ont déjà, le retirer)
g group (ajoute l'expéditeur dans un groupe)
l link (panneau des liens du mail ; entrée ouvre le lien dans le navigateur)
u lu / non lu
\* drapeau
/ rechercher ; entrée garde les résultats, j/k s'y déplacent, entrée ouvre le mail, échap efface la recherche
? liste des touches
échap annule un préfixe, ferme un panneau, démarque, puis revient du tag, groupe ou contact listé au dossier
espace, ctrl-d, ctrl-u, ctrl-f, ctrl-b font défiler le mail

Le prefix ";" permet de changer d’écran
; aller à un dossier
i Inbox
t Todo
w Waiting
a Archive
s Sent
r Drafts
n Newsletter
p Paperwork
q Quarantine
d Deleted
x Junk
b Banned

Les lettres sont celles des dossiers (`dossiers.md`) : `j` est réservé à la navigation, d'où `x` pour Junk, y compris après un préfixe.

ainsi ;i va à l’inbox, ;p va à Paperwork, ;t va à Todo, etc

Le prefix ";" permet également d'aller aux autres écrans : 
c Contact (liste des adresses ; / filtre ; entrée liste les mails du contact, tous dossiers)
h History (affichage de l'historique issu de l audit log, plus récent d'abord)
T tag (majuscule : `;t` est Todo ; entrée liste les mails du tag)
g group (entrée liste les mails des membres du groupe)

Sur ces écrans, j/k se déplacent et échap revient aux mails.


Il y a deux autres préfixes : s et v. (`d` était prévu pour display, mais `d` seul archive : une touche ne peut pas être à la fois une action et un préfixe.)
s save (dernier mail de la conversation, dans ~/Downloads ; un fichier existant n'est jamais écrasé)
	a all attachments (pas encore)
	g attachments but graphics (gif, png, ...) (pas encore)
	p as pdf (pas encore)
	e as eml file
	t as text file
	m as markdown file
v display (view)
	1 une seule colonne (masque la liste des dossiers et la lecture d'un mail)
	2 colonnes (presentation classique : liste des mails à gauche et lecture du mail à droite)
	3 colonnes (liste des dossiers à gauche, puis liste des mails au milieu et lecture du mail à droite)
	c creation date (mail trié par date de création, ou de réception)
	m modification date (mail trié par date de modification: permet de voir ce que l'on a modifié)
	h texte / HTML (choix mémorisé pour l'expéditeur)
	s barre de statut : en bas, en haut, masquée

Le nombre de colonnes, la position de la barre de statut et la largeur de la liste sont mémorisés.

Taille du texte, comme dans Zed : ⌘+ (ou ⌘=, sans Maj sur un clavier AZERTY) agrandit toute l'interface, ⌘− la réduit, ⌘0 revient à la taille par défaut, d'un point à la fois entre 9 et 24 points (12 par défaut). Une seule taille de base : polices, hauteur des lignes, largeur des colonnes, menus, sélecteurs et vue HTML en découlent. Elle est mémorisée.
