# NiceMailer : plan de travail de la première release

Statut : signature et notarisation écrites (`make release`, `docs/app-bundle.md`, « Signature » et « Releases ») ; Developer ID retenu. Restent : certificat Developer ID à créer, premier essai de notarisation, architecture, licence, README, recette.

Ce document n'est pas une spécification : c'est le plan pour rendre NiceMailer installable sur un autre Mac, depuis les GitHub Releases du dépôt `nicemailer/nicemailer`. Les règles restent dans `architecture.md` (section 10, « Distribution directe via GitHub ») et `mise-a-jour.md` ; quand ce plan tranche une question qu'ils laissaient ouverte, la décision y est reportée au moment où l'étape est livrée.

## 1. Constat

La chaîne de publication est décrite dans `architecture.md`, section 10, mais rien n'est construit :

- `scripts/make-bundle.sh` installe l'app dans `/Applications` et la signe avec le certificat **Apple Development** (`scripts/sign-core.sh`). Ce certificat ne peut pas être notarisé : sur un autre Mac, Gatekeeper bloque l'app.
- Le *hardened runtime* (`codesign --options runtime`), obligatoire pour la notarisation, n'est pas activé ; les signatures ne sont pas horodatées (`--timestamp`).
- `CFBundleShortVersionString` est figé à `0.1` ; `CFBundleVersion` vaut `git describe`, qui n'est pas un nombre croissant (Sparkle compare ce champ).
- Ni Sparkle ni `prepare_update` / `cancel_update` (`mise-a-jour.md`) ne sont implémentés : pas de mise à jour automatique.
- Le noyau ne vérifie pas encore la signature des clients (`architecture.md`, section 9) : changer d'identité de signature ne casse donc rien de ce côté.
- Binaires arm64 seulement.
- Le README est faux : il dit encore « no IMAP, no SMTP, no LaunchAgent and no code signing ».
- `Cargo.toml` déclare `license = "UNLICENSED"`.

## 2. Décisions à prendre

| Sujet | Options | Recommandation |
| --- | --- | --- |
| Signature | A. Sans compte payant : signature ad hoc, Gatekeeper à contourner à la main. B. Compte Apple Developer (99 €/an), certificat **Developer ID Application**, notarisation | **B.** Voir ci-dessous |
| Architecture | arm64 seul, ou binaires universels (`lipo`) | **arm64 seul** pour la v0.1, annoncé dans les notes de version. `architecture.md` (section 11) demande de trancher avant le premier release |
| Licence | Aucune (tous droits réservés), ou une licence libre | Dépend de la visibilité du dépôt : public sans licence, personne n'a le droit d'utiliser le code |

### Pourquoi Developer ID

| | Sans compte payant | Developer ID |
| --- | --- | --- |
| Premier lancement sur un autre Mac | Bloqué ; Réglages > Confidentialité et sécurité > « Ouvrir quand même », ou `xattr -dr com.apple.quarantine NiceMailer.app` | Double-clic, sans avertissement |
| Mots de passe dans le Keychain | Signature ad hoc → partition `cdhash:` : chaque nouvelle version perd les mots de passe IMAP enregistrés (`docs/configuration.md`, « Passwords ») | Partition `teamid:`, stable d'une version à l'autre |
| Mises à jour Sparkle (plus tard) | Fragiles | Prévues par `mise-a-jour.md` |

Pour une app de mail qui garde des mots de passe et tourne en agent de fond, seule l'option B est raisonnable, même pour quelques testeurs. L'option A ne convient qu'à une préversion pour deux ou trois personnes techniques, prévenues de ces deux défauts.

## 3. Travail dans le dépôt

1. **Mode release du bundle.** `make-bundle.sh --release <dossier>` (ou un script `scripts/make-release.sh`) :
   - construit le `.app` dans le dossier de sortie, sans l'installer ni toucher à l'app installée ;
   - signe de l'intérieur vers l'extérieur (helpers, puis app) avec l'identité Developer ID (`$SIGN_IDENTITY`), `--options runtime --timestamp` ;
   - garde les identifiants de signature actuels (`org.nicemailer.core`, `eu.nicemailer.cli`, `eu.nicemailer.mcp`), sur lesquels reposera la vérification des clients ; le noyau garde `org.nicemailer.core`, dont dépend l'accès aux mots de passe du trousseau ;
   - n'injecte aucune variable de développement dans l'agent (`NICEMAILER_FAKE_NETWORK`) ;
   - vérifie le résultat : `codesign --verify --deep --strict`, et refuse un arbre git modifié (`--dirty`).
2. **Versions.** Version tirée du tag `vX.Y.Z` → `CFBundleShortVersionString` ; `CFBundleVersion` entier croissant (par exemple le nombre de commits, `git rev-list --count HEAD`). Même version dans `Cargo.toml` (`workspace.package.version`) ; le CLI l'affiche (`nicemailer --version`).
3. **Hardened runtime.** Vérifier que tout marche avec : l'éditeur (WKWebView, qui tourne hors du processus et ne demande pas d'entitlement), l'accès aux Contacts, le Keychain, `SMAppService`, `launchctl kickstart`. Ajouter un fichier d'entitlements seulement si un essai le montre nécessaire.
4. **DMG.** `hdiutil create -format UDZO`, avec un lien vers `/Applications` ; DMG signé, notarisé, ticket agrafé.
5. **Schéma de la base.** À la publication, la version 1 du schéma est gelée (`docs/migrations.md`) : plus aucune modification en place. Taguer la release pour que la fixture de la version publiée puisse être reprise avec `git show v0.1.0:specifications/schema.sql`.
6. **Documentation.**
   - README : statut réel, installation depuis les Releases, prérequis (macOS 14, Apple Silicon), désinstallation.
   - `docs/app-bundle.md` : mode release, signature, notarisation.
   - `docs/configuration.md` : les mots de passe des builds de développement ne sont pas lisibles par l'app publiée si l'équipe Apple diffère ; il faut les ressaisir une fois.
   - Notes de version : pas de mise à jour automatique (remplacer l'app à la main ; le noyau est relancé par l'app), arm64 seul.
7. **Licence** (section 2).

## 4. Publication de la v0.1, à la main

La première release se fait depuis un script local, pas depuis la CI : la première notarisation échoue presque toujours sur un détail, bien plus vite corrigé en local que dans GitHub Actions avec des secrets en base64.

```sh
# une seule fois
xcrun notarytool store-credentials nicemailer --apple-id <apple-id> --team-id <team-id>
gh auth login

# à chaque release
scripts/make-bundle.sh --release dist/
hdiutil create -volname NiceMailer -srcfolder dist/NiceMailer.app -ov -format UDZO dist/NiceMailer-0.1.0.dmg
codesign --sign "Developer ID Application: …" --timestamp dist/NiceMailer-0.1.0.dmg
xcrun notarytool submit dist/NiceMailer-0.1.0.dmg --keychain-profile nicemailer --wait
xcrun stapler staple dist/NiceMailer-0.1.0.dmg
spctl -a -t open --context context:primary-signature -v dist/NiceMailer-0.1.0.dmg
git tag v0.1.0 && git push --tags
gh release create v0.1.0 dist/NiceMailer-0.1.0.dmg --prerelease --notes-file RELEASE_NOTES.md
```

En cas de refus de la notarisation : `xcrun notarytool log <submission-id> --keychain-profile nicemailer`.

## 5. Recette avant publication

Sur un autre compte utilisateur, ou mieux sur un autre Mac, à partir du DMG téléchargé depuis GitHub (pour qu'il porte l'attribut de quarantaine) :

- [ ] Ouverture sans avertissement de Gatekeeper.
- [ ] « Élément de fond ajouté » au premier lancement ; le noyau démarre (`launchctl print gui/$(id -u)/eu.nicemailer.core`).
- [ ] Ajout d'un compte IMAP réel ; synchronisation ; envoi d'un mail.
- [ ] Quitter l'app : le noyau continue ; la relancer : reconnexion sans délai.
- [ ] Redémarrage de la session : le noyau repart ; mots de passe lus sans invite du Keychain.
- [ ] Remplacement de l'app par une build plus récente (simulation d'une mise à jour manuelle) : le noyau est relancé avec le nouveau binaire, la base est migrée si besoin, les mots de passe restent lisibles.
- [ ] Lien `nicemailer://m/<id>` ; éditeur de texte ; accès aux Contacts.
- [ ] CLI du bundle : `NiceMailer.app/Contents/Helpers/nicemailer --version`.
- [ ] App mise à la corbeille : comportement de l'agent noté dans la doc (`architecture.md`, section 11).

## 6. Ensuite

1. **CI GitHub Actions** (runner macOS, déclenchée par un tag `vX.Y.Z`) reprenant le script de la section 4. Secrets : certificat `.p12` en base64 et son mot de passe, clé API App Store Connect pour `notarytool`.
2. **Sparkle** et `prepare_update` / `cancel_update` (`mise-a-jour.md`) : `appcast.xml` signé EdDSA, en asset de release ou sur GitHub Pages ; clé privée Sparkle en secret.
3. **Cask Homebrew** (`architecture.md`, section 10, « Canal complémentaire »).
4. **Vérification de la signature des clients** par le noyau (`architecture.md`, section 9), contre l'équipe Developer ID.
5. Binaires universels, si Intel est retenu.
