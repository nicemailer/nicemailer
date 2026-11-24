# NiceMailer : connexion OAuth2 (Microsoft 365 et Outlook.com)

Statut : brouillon, à valider. Complète `architecture.md` (section 6, « Secrets et Keychain »), `configuration.md` (section 6, « Secrets liés au serveur »), `plan-imap.md` (section 6) et `reponse.md` (section 7).

## 1. Objet

Ce document spécifie comment le noyau se connecte en IMAP et en SMTP à un compte qui n'accepte pas de mot de passe, en obtenant et en renouvelant lui-même un jeton OAuth2. La première cible est Microsoft : comptes professionnels et scolaires Microsoft 365 (Exchange Online), et comptes personnels Outlook.com, Hotmail et Live. Chez eux, l'authentification IMAP par mot de passe est coupée, et OAuth2 est le seul moyen de s'y connecter.

Le mécanisme est écrit pour accueillir d'autres fournisseurs. Gmail n'est pas une cible de ce document : ses scopes mail sont « restreints », et une diffusion publique exige une vérification par Google et un audit de sécurité annuel payant (`plan-imap.md`, section 6). Le mot de passe d'application reste le chemin de Gmail.

## 2. Périmètre

| Inclus | Exclu |
| --- | --- |
| IMAP et SMTP par SASL `XOAUTH2`, avec un jeton OAuth2 | Microsoft Graph (lecture des mails, `sendMail`, calendrier, Teams, réponses automatiques) |
| Comptes professionnels, scolaires et personnels Microsoft, par le tenant `common` | Boîtes partagées et délégation (`username` différent du compte connecté) |
| Flux « authorization code » avec PKCE, navigateur par défaut, redirection sur le loopback | Flux « device code », pour un Mac sans navigateur ou une session SSH |
| Renouvellement automatique des jetons, reconnexion demandée quand il échoue | Gmail par OAuth2 (section 1) |
| Ajout d'un compte et reconnexion, dans l'app et dans le CLI | Révocation de l'autorisation chez Microsoft (section 9) |

Graph n'est pas nécessaire pour lire et envoyer : IMAP et SMTP couvrent tout ce que fait le noyau aujourd'hui, et réutiliser IMAP réutilise tout le moteur de synchronisation. Graph devient utile si un tenant coupe IMAP ou SMTP AUTH (section 10) : c'est une question ouverte (section 13).

## 3. Principes

- **Le noyau fait tout, sauf ouvrir le navigateur.** Il prépare la requête d'autorisation, écoute la redirection, échange le code, garde les jetons et les renouvelle. Le client ouvre l'URL que le noyau lui donne, et c'est tout (`architecture.md`, section 6). Les jetons ne passent jamais par un client.
- **Aucun secret dans l'application.** Une app de bureau ne peut pas garder un secret : quiconque a le binaire le lit. L'application Microsoft est donc un **client public**, sans secret client, et PKCE (RFC 7636) protège l'échange du code.
- **Un jeton n'est envoyé qu'aux serveurs du fournisseur qui l'a délivré.** La liste de ces serveurs est compilée dans le noyau, et non lue dans `accounts.toml` (section 7). C'est la règle des secrets liés au serveur (`configuration.md`, section 6), appliquée aux jetons.
- **L'utilisateur voit toujours quel compte il autorise.** Le noyau demande le sélecteur de compte de Microsoft et vérifie que le compte autorisé est bien celui de l'adresse saisie (section 5).
- **Une panne de renouvellement n'est pas une panne de compte.** Une erreur réseau pendant un renouvellement est traitée comme toute erreur réseau, avec les mêmes reprises. Seul un refus explicite de Microsoft (`invalid_grant`, consentement retiré) met le compte en attente de reconnexion.

## 4. Inscription de l'application chez Microsoft

NiceMailer est inscrit une fois, par son auteur, dans Microsoft Entra ID. Cette inscription est gratuite et ne demande pas d'audit.

| Réglage | Valeur | Pourquoi |
| --- | --- | --- |
| Type de compte | Comptes dans n'importe quel annuaire d'organisation et comptes Microsoft personnels | Un seul identifiant pour Microsoft 365 et Outlook.com |
| Plateforme | Application mobile et de bureau | Client public, sans secret |
| URI de redirection | `http://127.0.0.1`, et `http://localhost` en second | Pour le loopback, Microsoft accepte n'importe quel port à l'exécution : le noyau peut prendre un port libre. Décision tranchée à l'inscription : `127.0.0.1`, que la documentation de Microsoft recommande plutôt que `localhost` ; `http://localhost` reste enregistré comme secours |
| « Autoriser les flux clients publics » | Oui | Requis pour un client sans secret |
| Permissions déléguées | `IMAP.AccessAsUser.All`, `SMTP.Send` (ressource Office 365 Exchange Online), `offline_access`, `openid`, `email`, `profile` | Les deux premières suffisent pour le mail ; `offline_access` donne le jeton de renouvellement ; les trois dernières, l'adresse du compte autorisé (section 5) |

Les deux permissions mail sont, en principe, accordables par l'utilisateur lui-même, sans administrateur. En pratique, beaucoup d'organisations interdisent ce consentement pour une application dont l'éditeur n'est pas vérifié (section 10). La vérification de l'éditeur demande un compte Microsoft Partner (MCPP) : c'est un préalable à une diffusion large, à décider par l'auteur (section 13).

### Identifiant de l'application

L'identifiant de l'application (« client ID ») n'est pas un secret : il est compilé dans le noyau, comme constante de `nm-engine`. Tant que l'application n'est pas inscrite dans Entra ID, la constante est un GUID nul fictif : aucun identifiant n'est alors disponible, les serveurs OAuth2-only ne sont pas utilisables (section 7) et `oauth_begin` échoue ; les builds de développement passent par le réglage ci-dessous.

Le réglage `oauth.microsoft_client_id` de `config.toml` le remplace. Il sert à l'auteur pour ses builds de développement, et à l'utilisateur dont l'organisation exige une application qu'elle a inscrite elle-même. Le noyau vérifie sa forme (un GUID non nul) à la lecture du fichier ; une valeur invalide fait refuser le fichier, comme toute valeur hors limites (`configuration.md`, section 4).

Les jetons sont liés à l'identifiant qui les a obtenus (section 6). Changer l'identifiant demande donc de reconnecter les comptes Microsoft : un jeton de renouvellement ne peut pas être présenté avec un autre identifiant d'application.

## 5. Flux d'autorisation

Le flux est le « authorization code flow » d'OAuth2 (RFC 6749), avec PKCE et une redirection sur le loopback (RFC 8252, section 7.3).

1. Le client appelle `oauth_begin` avec l'adresse du compte à créer, ou la clé d'un compte existant (section 8).
2. Le noyau :
   - ouvre une écoute sur `127.0.0.1` et `[::1]`, sur un port libre choisi par le système, le même pour les deux ; `localhost` peut se résoudre vers l'une ou l'autre adresse selon le navigateur ;
   - tire un `code_verifier` (32 octets aléatoires, base64url), un `state` et un `nonce` (16 octets aléatoires chacun) ;
   - construit l'URL d'autorisation : `https://login.microsoftonline.com/common/oauth2/v2.0/authorize`, avec `response_type=code`, `client_id`, `redirect_uri=http://127.0.0.1:<port>`, les scopes de la section 4, `state`, `nonce`, `code_challenge` (SHA-256 du verifier) et `code_challenge_method=S256`, `login_hint` (l'adresse) et `prompt=select_account` ;
   - renvoie l'URL et un identifiant de flux.
3. Le client ouvre l'URL dans le **navigateur par défaut**. Le navigateur de l'utilisateur garde ses sessions, ses clés d'accès et son gestionnaire de mots de passe ; une vue web intégrée n'aurait rien de cela, et apprendrait à l'utilisateur à taper son mot de passe Microsoft dans une fenêtre qui n'est pas celle de Microsoft.
4. L'utilisateur se connecte et accepte. Microsoft redirige le navigateur vers `http://127.0.0.1:<port>/?code=…&state=…`, ou `?error=…&error_description=…` s'il a refusé.
5. Le noyau reçoit cette requête et ferme l'écoute. Il ne traite qu'**une seule** requête `GET /` ; toute autre requête (autre chemin, autre méthode, favicon) reçoit `404` et ne termine pas le flux. Si `state` diffère de celui tiré, le flux échoue (`oauth_state_mismatch`) : la requête ne vient pas de cette autorisation. Le navigateur reçoit une page courte, sans ressource externe, qui dit que l'autorisation a réussi ou échoué et que l'onglet peut être fermé. Le noyau l'écrit en anglais ou en français, selon l'en-tête `Accept-Language`, en anglais par défaut.
6. Le noyau échange le code contre les jetons sur `https://login.microsoftonline.com/common/oauth2/v2.0/token` (`grant_type=authorization_code`, `code`, `redirect_uri`, `client_id`, `code_verifier`). Il reçoit un jeton d'accès (valable environ une heure), un jeton de renouvellement et un `id_token`.
7. Le noyau lit l'`id_token`. Il vérifie le `nonce`, puis compare l'adresse du compte autorisé (`email`, sinon `preferred_username`) à l'adresse du compte, sans tenir compte de la casse. Si elles diffèrent, le flux échoue (`oauth_wrong_account`), avec les deux adresses : l'utilisateur a choisi un autre compte dans le sélecteur. Il garde aussi l'identifiant du tenant (`tid`) et celui de l'utilisateur (`oid`, sinon `sub`).

   Le noyau ne vérifie pas la signature de l'`id_token` : il le reçoit directement de Microsoft par TLS, en réponse à sa propre requête, et OpenID Connect l'en dispense dans ce cas (OpenID Connect Core, section 3.1.3.7). L'`id_token` ne sert qu'à afficher et comparer l'adresse ; c'est le serveur IMAP qui décide de l'accès.
8. Le noyau émet `OAuthFinished {flow, ok, why}` (section 8). Les jetons restent en mémoire, attachés au flux, jusqu'à ce que `add_account` les enregistre ; pour un compte existant, ils sont enregistrés tout de suite.

### Limites du flux

- **Durée.** Un flux expire après 10 minutes sans redirection : l'écoute est fermée et `OAuthFinished` signale `oauth_timeout`. Les jetons d'un flux réussi que personne n'a utilisés sont oubliés après 10 minutes.
- **Un flux à la fois par adresse.** Un nouvel `oauth_begin` pour la même adresse ou le même compte annule le précédent (`oauth_cancelled`). Des flux pour des adresses différentes peuvent coexister, chacun sur son port.
- **Annulation.** `oauth_cancel` ferme l'écoute et oublie le flux. Le client l'appelle quand l'utilisateur abandonne l'ajout du compte.
- **Requête lente ou hostile.** L'écoute n'accepte que le loopback. Elle lit au plus 8 Kio d'en-têtes, en 5 secondes au plus, et coupe au-delà. Un programme local qui devine le port ne peut rien obtenir : sans le bon `state`, il ne termine pas le flux, et le code ne vaut rien sans le `code_verifier`, qui ne quitte jamais le noyau.

## 6. Jetons et Keychain

### Ce qui est enregistré

Le Keychain garde, pour chaque compte OAuth2, une entrée à la place du mot de passe IMAP, sous le même nom dérivé de la clé du compte. Elle contient :

- le jeton de renouvellement ;
- le fournisseur (`microsoft`) et l'identifiant d'application qui l'a obtenu ;
- le tenant (`tid`), l'utilisateur (`oid`) et l'adresse autorisée ;
- comme pour un mot de passe, l'hôte et l'identifiant IMAP pour lesquels elle a été créée (`configuration.md`, section 6).

Le jeton d'accès n'est **jamais** écrit sur le disque. Il vit en mémoire dans le noyau, avec son heure d'expiration. Au redémarrage du noyau, le premier accès en obtient un nouveau. Le perdre ne coûte qu'un renouvellement, alors que l'écrire l'exposerait à toute copie du trousseau.

Un seul jeton d'accès sert à IMAP et à SMTP : les deux permissions appartiennent à la même ressource (`outlook.office.com`). Microsoft délivre des jetons valables pour **une seule ressource** ; un futur scope Graph demanderait donc un second jeton d'accès, obtenu avec le même jeton de renouvellement, et jamais mélangé au premier.

### Renouvellement

- **Quand.** Le noyau renouvelle le jeton d'accès quand il lui reste moins de 5 minutes, au moment où une connexion en a besoin, et non par une minuterie : un compte sans activité ne sollicite pas Microsoft.
- **Un renouvellement à la fois par compte.** Les connexions d'un compte (dossiers, corps, IDLE, SMTP : `sync.max_connections`) peuvent avoir besoin d'un jeton au même instant. Le premier appel renouvelle ; les autres attendent son résultat au lieu de lancer leur propre requête. Sans cela, des renouvellements simultanés feraient tourner le jeton de renouvellement plusieurs fois et pourraient en perdre un.
- **Rotation.** Microsoft peut renvoyer un nouveau jeton de renouvellement avec chaque jeton d'accès. Le noyau enregistre le nouveau dans le Keychain **avant** de se servir du jeton d'accès. Si l'écriture échoue (trousseau verrouillé), il utilise quand même le jeton d'accès et garde le nouveau jeton de renouvellement en mémoire, réessaie de l'écrire au renouvellement suivant, et le consigne dans le journal. L'ancien jeton reste en général valable, mais le noyau ne compte pas dessus.
- **Durée de vie.** Un jeton de renouvellement Microsoft expire après environ 90 jours sans usage. Un compte synchronisé le renouvelle bien avant. Un compte resté éteint plus longtemps demande une reconnexion, ce qui est normal.
- **Refus d'un jeton d'accès par le serveur.** Si IMAP ou SMTP refuse un jeton d'accès que le noyau croyait valide (révoqué, mot de passe changé, accès conditionnel), le noyau en obtient un nouveau et réessaie **une fois**. Un second refus est un refus du serveur (section 10).

### Erreurs du point de terminaison des jetons

| Réponse de Microsoft | Traitement | Raison |
| --- | --- | --- |
| Erreur réseau, `5xx`, `temporarily_unavailable` | Reprise avec les délais habituels des erreurs réseau ; le compte n'est pas mis en erreur | — |
| `invalid_grant` (jeton expiré, révoqué, mot de passe changé) ; `interaction_required` (MFA ou accès conditionnel à refaire) | Le compte attend une reconnexion ; sa synchronisation est suspendue | `oauth_reauth_needed` |
| `AADSTS65001` ou `AADSTS90094` (consentement absent, ou réservé à l'administrateur) | Le compte attend une reconnexion ; l'app affiche le lien de consentement administrateur (section 10) | `oauth_admin_consent_needed` |
| `AADSTS53003` (bloqué par l'accès conditionnel) | Le compte est en erreur ; reconnecter ne servirait à rien tant que l'organisation ne change pas sa règle | `oauth_blocked_by_policy` |
| `invalid_client`, `unauthorized_client` | Le compte est en erreur : l'identifiant d'application est mauvais ou désactivé | `oauth_client_rejected` |

Le noyau cite le code `AADSTS…` et le texte de Microsoft dans le journal et dans `why`, en anglais, à l'intérieur de la phrase localisée de l'app : c'est ce qu'un administrateur demandera.

## 7. IMAP et SMTP

### SASL XOAUTH2

Le noyau s'authentifie par le mécanisme `XOAUTH2`, que Microsoft documente pour IMAP, POP et SMTP. La réponse du client est la chaîne `user={utilisateur}\x01auth=Bearer {jeton}\x01\x01`, encodée en base64. `{utilisateur}` est le `username` du serveur dans `accounts.toml`, par défaut l'adresse du compte.

- **IMAP.** `AUTHENTICATE XOAUTH2`, après vérification de la capacité `AUTH=XOAUTH2`. La réponse part dans la commande si le serveur annonce `SASL-IR` (RFC 4959), sinon après la première continuation `+`. Si le serveur répond par une seconde continuation (un défi qui décrit l'erreur, en JSON base64), le noyau envoie une ligne vide et attend le `NO` final, qu'il traite comme un refus. Comme pour le mot de passe, les capacités sont relues après l'authentification (`plan-imap.md`, section 1).
- **SMTP.** `AUTH XOAUTH2 <réponse>` après `STARTTLS` (port 587 de `smtp.office365.com`). Une réponse `334` est le même défi d'erreur : le noyau répond par une ligne vide et attend le `535`.

Le module SASL de `nm-proto` reçoit le jeton comme un `SecretString` et ne l'écrit jamais dans le journal, ni en clair ni encodé. Les traces de protocole remplacent la ligne d'authentification par `[jeton masqué]`, comme elles le font pour un mot de passe.

`OAUTHBEARER` (RFC 7628) est le mécanisme standard. Microsoft ne le documente pas pour Exchange Online : le noyau ne l'utilise pas pour Microsoft, mais le module SASL est écrit pour l'ajouter sans changer de structure.

### Serveurs autorisés

Le fournisseur d'un compte OAuth2 se déduit de son serveur IMAP, par une table compilée dans le noyau :

| Fournisseur | IMAP | SMTP |
| --- | --- | --- |
| `microsoft` | `outlook.office365.com` | `smtp.office365.com`, `smtp-mail.outlook.com` |

Le noyau n'envoie un jeton qu'à un hôte de cette table, en TLS, avec le certificat vérifié. Un compte `auth = "oauth2"` dont le serveur n'y figure pas est en erreur (`oauth2_unknown_server`), et aucun jeton n'est demandé. Lire cette liste dans `accounts.toml` permettrait à quiconque peut modifier le fichier de recevoir un jeton valable une heure sur toute la boîte : c'est exactement ce que la règle des secrets liés au serveur interdit pour un mot de passe (`configuration.md`, section 6).

La liaison de l'entrée du Keychain à l'hôte et à l'identifiant s'applique comme pour un mot de passe : si `accounts.toml` désigne un autre identifiant IMAP que celui de l'entrée, le noyau refuse de s'authentifier et demande une reconnexion.

### Découverte

`discover_account` reconnaît Microsoft :

- par le domaine de l'adresse (`outlook.com`, `hotmail.com`, `live.com`, `msn.com` et leurs variantes nationales, d'après la base de fournisseurs) ;
- sinon par le MX : un domaine dont le MX se termine par `.mail.protection.outlook.com` est hébergé par Microsoft 365.

Il renvoie alors `auth: "oauth2"` et `oauth_provider: "microsoft"`, avec les serveurs de la table. `Server::usable` accepte désormais un serveur qui ne prend qu'OAuth2, si son fournisseur est dans la table et qu'un identifiant d'application est disponible.

## 8. Contrat et CLI

### Méthodes

| Méthode | Rôle |
| --- | --- |
| `oauth_begin {email}` ou `{account}` | Démarre un flux (section 5) ; renvoie `{flow, url, provider}`. Refuse une adresse dont le fournisseur n'accepte pas OAuth2 (`oauth2_unsupported_provider`) |
| `oauth_cancel {flow}` | Abandonne un flux ; sans effet sur un flux terminé |
| `test_account`, `test_smtp`, `add_account` | Acceptent `oauth: {flow}` à la place de `password`. Le flux doit être réussi, et pour la même adresse. `add_account` enregistre les jetons du flux dans le Keychain, puis les oublie en mémoire |
| `remove_account` | Supprime aussi l'entrée OAuth2 du Keychain et le jeton d'accès en mémoire |

Pour un compte existant, `oauth_begin {account}` remplace `set_password` : à la fin d'un flux réussi, le noyau enregistre les nouveaux jetons, efface l'erreur du compte et relance sa synchronisation.

### Événement

`OAuthFinished {flow, ok, account?, email, why?}` : la fin d'un flux, réussi ou non (`oauth_denied` si l'utilisateur a refusé chez Microsoft, `oauth_timeout`, `oauth_cancelled`, `oauth_state_mismatch`, `oauth_wrong_account`, ou une raison de la section 6). Le client n'a pas à interroger le noyau pendant que l'utilisateur est dans son navigateur.

### Raisons

Les raisons nouvelles (`oauth_*`, `smtp_auth_disabled`, `imap_disabled_for_mailbox`) suivent la règle du contrat : un code, des valeurs et une phrase anglaise, que les clients écrivent dans la langue de l'utilisateur. `oauth2_unsupported` disparaît du contrat : un compte `auth = "oauth2"` dont le serveur est hors de la table de la section 7 a la raison `oauth2_unknown_server` (une situation, une raison). `oauth2_unsupported_provider` reste, pour `oauth_begin` sur une adresse dont le fournisseur n'accepte pas OAuth2.

### CLI

- `nicemailer account add` : quand la découverte renvoie `oauth2`, le CLI n'invite pas à saisir de mot de passe. Il appelle `oauth_begin`, affiche l'URL, l'ouvre avec `open` et attend `OAuthFinished`, en affichant « Waiting for sign-in in your browser… ». Avec `--no-browser`, il affiche l'URL sans l'ouvrir. Ctrl-C appelle `oauth_cancel`.
- `nicemailer account login <key>` : reconnecte un compte OAuth2. Sur un compte par mot de passe, il renvoie vers `account password`.

La redirection arrive sur le loopback du Mac qui exécute le noyau : par SSH, il faut ouvrir l'URL dans un navigateur de ce Mac. C'est la limite du flux, et la raison du flux « device code » mis hors périmètre (section 2).

`docs/contract.md`, `docs/` et `CHANGELOG.md` sont mis à jour dans le même changement que le code.

## 9. Interface de l'app

- **Ajout d'un compte.** L'écran garde ses trois champs (`ui.md`, « Ajout d'un compte »). Quand l'essai découvre un compte Microsoft, le champ du mot de passe est remplacé par un bouton « Se connecter avec Microsoft », avec une phrase : la connexion se fait dans le navigateur, et NiceMailer ne voit jamais le mot de passe. Un mot de passe déjà tapé n'est pas envoyé. Le bouton appelle `oauth_begin`, ouvre l'URL (`NSWorkspace.open`) et affiche « En attente de la connexion dans le navigateur… », avec un bouton « Annuler » (`oauth_cancel`) et un lien « Rouvrir la page » qui rouvre la même URL. À `OAuthFinished` réussi, l'ajout reprend là où il s'était arrêté (essai IMAP, SMTP, import) ; en échec, l'écran affiche la raison et rend le bouton.
- **Compte à reconnecter.** Un compte en `oauth_reauth_needed` affiche le bandeau d'erreur de compte habituel, avec un bouton « Se reconnecter » qui lance le même flux pour ce compte.
- **Consentement administrateur.** Pour `oauth_admin_consent_needed`, l'app explique que l'organisation doit approuver NiceMailer, et propose de copier le lien de consentement administrateur, `https://login.microsoftonline.com/{tid}/adminconsent?client_id={client_id}`, à transmettre à l'administrateur.
- **Suppression d'un compte.** Microsoft n'offre pas, à un client public, de moyen de révoquer un jeton de renouvellement. Supprimer le compte efface les jetons du Mac. La confirmation l'indique : l'autorisation reste visible et révocable sur la page « Applications » du compte Microsoft, ou dans « Mes applications » pour un compte professionnel.

Toutes ces chaînes passent par le String Catalog, en anglais et en français.

## 10. Ce qui peut bloquer, et comment le dire

Un code OAuth2 correct ne suffit pas toujours. Ces refus viennent de l'organisation, pas de NiceMailer, et l'utilisateur doit savoir à qui s'adresser.

| Situation | Ce que voit le noyau | Raison et message |
| --- | --- | --- |
| L'organisation interdit aux utilisateurs de consentir aux applications d'éditeurs non vérifiés | Page « Approbation de l'administrateur requise » chez Microsoft, puis `AADSTS65001` ou `AADSTS90094` | `oauth_admin_consent_needed`, avec le lien de consentement administrateur |
| IMAP désactivé pour la boîte, ou boîte sans licence Exchange | `AUTHENTICATE failed` sur un jeton **tout juste obtenu** | `imap_disabled_for_mailbox` : demander à l'administrateur d'activer IMAP pour la boîte. Exchange ne distingue pas ce cas d'un jeton refusé : le noyau ne le conclut qu'après un échec avec un jeton neuf |
| SMTP AUTH désactivé pour le tenant ou la boîte (`SmtpClientAuthenticationDisabled`) | `535 5.7.139 … SmtpClientAuthentication is disabled …` | `smtp_auth_disabled` : le compte reste utilisable en lecture, sans envoi, comme un compte sans `[smtp]` ; demander à l'administrateur d'activer SMTP AUTH pour la boîte |
| Accès conditionnel (appareil géré exigé, pays bloqué) | `AADSTS53003`, ou refus à la connexion | `oauth_blocked_by_policy` |

Microsoft a annoncé la fin de l'authentification de base (mot de passe) pour SMTP AUTH dans Exchange Online ; la date est à vérifier. Une fois en vigueur, OAuth2 sera aussi le seul moyen d'envoyer par SMTP pour un compte Microsoft.

## 11. Configuration

- **`accounts.toml`.** Un compte Microsoft porte `auth = "oauth2"`, et ses serveurs sont ceux de la table de la section 7. Aucune autre clé : le fournisseur se déduit du serveur. Un compte `oauth2` déclaré à la main reste en erreur `oauth_reauth_needed` jusqu'à la première connexion, comme un compte par mot de passe sans mot de passe (`configuration.md`, section 6, « Ajout d'un compte »).
- **`config.toml`.** Une clé, `oauth.microsoft_client_id`, vide par défaut : l'identifiant compilé est alors utilisé (section 4).
- **Base.** Rien ne change dans le schéma : les jetons sont dans le Keychain, et l'état d'erreur d'un compte existe déjà.

## 12. Tests exigés

Les tests ne touchent jamais Microsoft. Les adresses des points de terminaison (autorisation et jetons) sont injectables dans le noyau de test, comme les serveurs IMAP et SMTP de test.

- **SASL.** Encodage de la réponse XOAUTH2 ; IMAP avec et sans `SASL-IR` ; défi d'erreur suivi d'une ligne vide, en IMAP et en SMTP ; capacités relues après l'authentification ; le jeton n'apparaît dans aucune trace ni aucun journal.
- **Flux.** Contre un faux serveur d'autorisation : flux réussi de bout en bout ; `state` faux ; requête sur un autre chemin, puis la bonne ; `error=access_denied` ; expiration après 10 minutes (horloge simulée) ; annulation ; nouveau flux pour la même adresse qui annule l'ancien ; `id_token` d'une autre adresse ; `nonce` faux ; en-têtes trop longs ou trop lents ; l'écoute n'est ouverte que sur le loopback.
- **Jetons.** Renouvellement avant expiration ; dix connexions simultanées ne déclenchent qu'un renouvellement ; rotation enregistrée avant usage ; écriture du Keychain qui échoue ; jeton refusé par IMAP, renouvelé et réessayé une seule fois ; chaque ligne du tableau d'erreurs de la section 6 donne la bonne raison et le bon état de compte ; une erreur réseau ne met pas le compte en erreur.
- **Sécurité.** `accounts.toml` modifié pour pointer vers un autre hôte : aucun jeton envoyé, ni demandé ; identifiant IMAP changé : reconnexion demandée ; changement de `oauth.microsoft_client_id` : reconnexion demandée.
- **Découverte.** Domaine Outlook.com ; domaine personnalisé avec MX `*.mail.protection.outlook.com` ; domaine sans lien avec Microsoft.
- **Contrat et CLI.** `oauth_begin`, `oauth_cancel`, `add_account` avec `oauth`, `OAuthFinished` ; `account add` et `account login` avec un faux fournisseur ; `docs/contract.md` à jour (tests de `contract_doc.rs`).
- **App.** Test unitaire et scénario `.uiscript` pour : bouton Microsoft à la place du mot de passe, attente puis reprise de l'ajout, annulation, bandeau de reconnexion, message de consentement administrateur (`CLAUDE.md`, « Testing the app »). Le scénario ne doit pas ouvrir de vrai navigateur : en mode script, l'ouverture d'URL est remplacée par un enregistrement que `Runner.state()` expose.
- **Essai réel.** Par l'auteur, sur un compte Microsoft 365 et un compte Outlook.com : ajout, synchronisation, envoi, reconnexion après révocation de l'autorisation, noyau redémarré après plus d'une heure.

## 13. Questions ouvertes

- **Vérification de l'éditeur.** Sans elle, de nombreuses organisations bloqueront le consentement (section 10). Elle demande un compte Microsoft Partner (MCPP) et un domaine vérifié. À décider avant une diffusion publique ; d'ici là, NiceMailer est utilisable sur les comptes personnels et dans les organisations qui autorisent le consentement.
- ~~**`localhost` ou `127.0.0.1`** dans l'URI de redirection~~ : tranché — `http://127.0.0.1`, avec `http://localhost` enregistré en secours (section 4).
- **Graph comme repli** quand un tenant coupe IMAP ou SMTP AUTH : envoi par `sendMail`, puis lecture. Le second moteur de synchronisation que cela implique n'est justifié que si le cas se révèle fréquent.
- **Boîtes partagées** : XOAUTH2 accepte un `user` différent du compte connecté, si celui-ci a les droits sur la boîte. Cela demande de séparer, dans `accounts.toml`, le compte qui s'autorise de la boîte lue.
- **Flux « device code »**, pour un noyau sans navigateur à portée.
- **Gmail par OAuth2**, si la vérification de Google devient envisageable : le mécanisme de ce document s'y applique, avec un client « Desktop » de Google, qui a un secret non confidentiel.
