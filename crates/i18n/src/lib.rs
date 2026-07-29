use std::borrow::Cow;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum Locale {
    #[default]
    English,
    French,
    Spanish,
}

impl Locale {
    pub const ALL: [Self; 3] = [Self::English, Self::French, Self::Spanish];

    pub const fn tag(self) -> &'static str {
        match self {
            Self::English => "en-US",
            Self::French => "fr-FR",
            Self::Spanish => "es",
        }
    }

    pub const fn native_name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::French => "Français",
            Self::Spanish => "Español",
        }
    }

    pub fn from_tag(tag: &str) -> Option<Self> {
        let language = tag
            .trim()
            .split(['-', '_'])
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        match language.as_str() {
            "en" => Some(Self::English),
            "fr" => Some(Self::French),
            "es" => Some(Self::Spanish),
            _ => None,
        }
    }

    pub fn resolve(saved: Option<&str>, system: Option<&str>) -> Self {
        saved
            .and_then(Self::from_tag)
            .or_else(|| system.and_then(Self::from_tag))
            .unwrap_or_default()
    }
}

macro_rules! catalog {
    ($locale:expr, $source:expr, { $($english:literal => ($french:literal, $spanish:literal)),* $(,)? }) => {
        match $source {
            $(
                $english => match $locale {
                    Locale::English => $english,
                    Locale::French => $french,
                    Locale::Spanish => $spanish,
                },
            )*
            other => other,
        }
    };
}

pub fn text<'a>(locale: Locale, source: &'a str) -> Cow<'a, str> {
    Cow::Borrowed(catalog!(locale, source, {
        // Common controls and states.
        "On" => ("Activé", "Activado"),
        "Off" => ("Désactivé", "Desactivado"),
        "Ready" => ("Prêt", "Listo"),
        "Missing" => ("Manquante", "Falta"),
        "Waiting" => ("En attente", "Esperando"),
        "Matching" => ("Correspondance", "Coincidencia"),
        "Not matching" => ("Aucune correspondance", "No coincide"),
        "Reference missing" => ("Référence manquante", "Falta la referencia"),
        "Initializing" => ("Initialisation", "Inicializando"),
        "Unavailable" => ("Indisponible", "No disponible"),
        "Failed" => ("Échec", "Error"),
        "Automatic" => ("Automatique", "Automático"),
        "Camera" => ("Caméra", "Cámara"),
        "Display" => ("Écran", "Pantalla"),
        "Webcam" => ("Webcam", "Cámara web"),
        "Screen" => ("Écran", "Pantalla"),
        "Output" => ("Sortie", "Salida"),
        "Reference" => ("Référence", "Referencia"),
        "Detection" => ("Détection", "Detección"),
        "Live" => ("EN DIRECT", "EN DIRECTO"),
        "LIVE" => ("EN DIRECT", "EN DIRECTO"),
        "WEBCAM" => ("WEBCAM", "CÁMARA WEB"),
        "SCREEN" => ("ÉCRAN", "PANTALLA"),
        "REFERENCE" => ("RÉFÉRENCE", "REFERENCIA"),
        "OUTPUT" => ("SORTIE", "SALIDA"),
        "Unknown" => ("Inconnu", "Desconocido"),
        "Crossfading" => ("Fondu enchaîné", "Fundido cruzado"),
        "Screen mix" => ("Mixage écran", "Mezcla de pantalla"),
        "Auto" => ("Auto", "Auto"),
        "Close" => ("Fermer", "Cerrar"),
        "Cancel" => ("Annuler", "Cancelar"),
        "Back" => ("Retour", "Atrás"),
        "Continue" => ("Continuer", "Continuar"),
        "Skip" => ("Ignorer", "Omitir"),

        // Dashboard and previews.
        "Settings" => ("Paramètres", "Ajustes"),
        "Components health" => ("État des composants", "Estado de los componentes"),
        "Main controls" => ("Commandes principales", "Controles principales"),
        "Start automation" => ("Démarrer l’automatisation", "Iniciar automatización"),
        "Stop automation" => ("Arrêter l’automatisation", "Detener automatización"),
        "Stopping automation…" => ("Arrêt de l’automatisation…", "Deteniendo automatización…"),
        "Output mode" => ("Mode de sortie", "Modo de salida"),
        "Other" => ("Autres actions", "Otras acciones"),
        "Capture reference" => ("Capturer la référence", "Capturar referencia"),
        "Rescan screens" => ("Rechercher les écrans", "Buscar pantallas"),
        "No webcam frame" => ("Aucune image de webcam", "No hay imagen de la cámara web"),
        "No screen frame" => ("Aucune image d’écran", "No hay imagen de pantalla"),
        "No reference image" => ("Aucune image de référence", "No hay imagen de referencia"),
        "No output frame" => ("Aucune image de sortie", "No hay imagen de salida"),
        "Preparing preview…" => ("Préparation de l’aperçu…", "Preparando vista previa…"),

        // Settings shell.
        "General" => ("Général", "General"),
        "Diagnostics" => ("Diagnostic", "Diagnóstico"),
        "Choose how the app launches, stays available, and reports problems." => ("Choisissez le mode de démarrage de l’application, sa disponibilité et le signalement des problèmes.", "Elige cómo se inicia la aplicación, cómo permanece disponible y cómo informa de los problemas."),
        "Select, verify, and recover the camera used for webcam output." => ("Sélectionnez, vérifiez et rétablissez la caméra utilisée pour la sortie webcam.", "Selecciona, comprueba y recupera la cámara utilizada para la salida de cámara web."),
        "Choose the display Automatic mode watches and how it is captured." => ("Choisissez l’écran surveillé par le mode Automatique et la façon dont il est capturé.", "Elige la pantalla que vigila el modo Automático y cómo se captura."),
        "Teach Automatic mode when the screen should show the webcam." => ("Indiquez au mode Automatique quand l’écran doit afficher la webcam.", "Indica al modo Automático cuándo debe mostrarse la cámara web."),
        "Inspect component health, technical details, logs, and recovery tools." => ("Consultez l’état des composants, les détails techniques, les journaux et les outils de récupération.", "Consulta el estado de los componentes, los detalles técnicos, los registros y las herramientas de recuperación."),
        "PREFERENCES" => ("PRÉFÉRENCES", "PREFERENCIAS"),
        "Back to dashboard" => ("Retour au tableau de bord", "Volver al panel"),
        "AUTOSAVE" => ("ENREGISTREMENT AUTO.", "GUARDADO AUTOMÁTICO"),
        "Saved" => ("Enregistré", "Guardado"),
        "Saving…" => ("Enregistrement…", "Guardando…"),
        "Couldn’t save" => ("Échec de l’enregistrement", "No se pudo guardar"),

        // General settings.
        "How StageSwap works" => ("Fonctionnement de StageSwap", "Cómo funciona StageSwap"),
        "StageSwap watches your selected screen. While it matches your saved reference image, your video calls see your webcam. When the screen changes, StageSwap automatically switches to the screen. When the reference returns, it switches back to your webcam." => ("StageSwap surveille l’écran sélectionné. Tant qu’il correspond à l’image de référence enregistrée, vos appels vidéo voient votre webcam. Lorsque l’écran change, StageSwap bascule automatiquement vers l’écran. Lorsque la référence réapparaît, il revient à votre webcam.", "StageSwap vigila la pantalla seleccionada. Mientras coincida con la imagen de referencia guardada, las videollamadas verán tu cámara web. Cuando cambie la pantalla, StageSwap cambiará automáticamente a ella. Cuando vuelva la referencia, regresará a la cámara web."),
        "Startup" => ("Démarrage", "Inicio"),
        "Applied the next time StageSwap starts." => ("Appliqué au prochain démarrage de StageSwap.", "Se aplica la próxima vez que se inicie StageSwap."),
        "Start with Windows" => ("Démarrer avec Windows", "Iniciar con Windows"),
        "Install StageSwap to use a stable Windows startup path." => ("Installez StageSwap pour utiliser un chemin de démarrage Windows stable.", "Instala StageSwap para usar una ruta de inicio de Windows estable."),
        "Install StageSwap to enable startup" => ("Installer StageSwap pour activer le démarrage", "Instalar StageSwap para activar el inicio"),
        "Launch after Windows sign-in." => ("Lancer après la connexion à Windows.", "Iniciar después de entrar en Windows."),
        "Start minimized" => ("Démarrer réduit", "Iniciar minimizado"),
        "Open in the system tray." => ("Ouvrir dans la zone de notification.", "Abrir en la bandeja del sistema."),
        "Start automation on launch" => ("Démarrer l’automatisation au lancement", "Iniciar la automatización al abrir"),
        "Window behavior" => ("Comportement de la fenêtre", "Comportamiento de la ventana"),
        "Choose what closing StageSwap does." => ("Choisissez l’action effectuée à la fermeture de StageSwap.", "Elige qué ocurre al cerrar StageSwap."),
        "Close window to tray" => ("Réduire la fenêtre dans la zone de notification", "Cerrar la ventana en la bandeja"),
        "Keep StageSwap running after closing the window." => ("Laisser StageSwap actif après la fermeture de la fenêtre.", "Mantener StageSwap en ejecución después de cerrar la ventana."),
        "Confirm before exit" => ("Confirmer avant de quitter", "Confirmar antes de salir"),
        "Ask before StageSwap fully exits." => ("Demander confirmation avant de quitter complètement StageSwap.", "Preguntar antes de cerrar StageSwap por completo."),
        "Notifications" => ("Notifications", "Notificaciones"),
        "Important Windows warnings." => ("Alertes Windows importantes.", "Avisos importantes de Windows."),
        "Show status notifications" => ("Afficher les notifications d’état", "Mostrar notificaciones de estado"),
        "Notify when a component needs attention." => ("Notifier lorsqu’un composant nécessite votre attention.", "Avisar cuando un componente requiera atención."),
        "Language" => ("Langue", "Idioma"),
        "Choose the language used by StageSwap." => ("Choisissez la langue utilisée par StageSwap.", "Elige el idioma que utiliza StageSwap."),
        "Interface language" => ("Langue de l’interface", "Idioma de la interfaz"),
        "Changes apply immediately." => ("Les modifications s’appliquent immédiatement.", "Los cambios se aplican de inmediato."),
        "On — Starts in {0} mode {1}." => ("Activé — Démarre en mode {0} {1}.", "Activado: se inicia en modo {0} {1}."),
        "in the tray" => ("dans la zone de notification", "en la bandeja"),
        "after the dashboard opens" => ("après l’ouverture du tableau de bord", "después de abrir el panel"),
        "Off — Shows the StageSwap off screen until automation starts." => ("Désactivé — Affiche l’écran d’arrêt de StageSwap jusqu’au démarrage de l’automatisation.", "Desactivado: muestra la pantalla de StageSwap desactivado hasta que se inicie la automatización."),
        "Closing hides the window; Exit from the tray asks for confirmation." => ("La fermeture masque la fenêtre ; Quitter depuis la zone de notification demande confirmation.", "Al cerrar se oculta la ventana; Salir desde la bandeja pide confirmación."),
        "Closing hides the window; Exit from the tray stops StageSwap immediately." => ("La fermeture masque la fenêtre ; Quitter depuis la zone de notification arrête immédiatement StageSwap.", "Al cerrar se oculta la ventana; Salir desde la bandeja detiene StageSwap de inmediato."),
        "Closing the window or choosing Exit asks before StageSwap stops." => ("Fermer la fenêtre ou choisir Quitter demande confirmation avant l’arrêt de StageSwap.", "Cerrar la ventana o elegir Salir pide confirmación antes de detener StageSwap."),
        "Closing the window or choosing Exit stops StageSwap immediately." => ("Fermer la fenêtre ou choisir Quitter arrête immédiatement StageSwap.", "Cerrar la ventana o elegir Salir detiene StageSwap de inmediato."),

        // Webcam and screen settings.
        "No camera selected" => ("Aucune caméra sélectionnée", "No hay ninguna cámara seleccionada"),
        "Saved camera is unavailable" => ("La caméra enregistrée est indisponible", "La cámara guardada no está disponible"),
        "Camera input" => ("Entrée caméra", "Entrada de cámara"),
        "Used by Camera mode and whenever Automatic selects Camera. Output is always 16:9." => ("Utilisée par le mode Caméra et lorsque le mode Automatique sélectionne la caméra. La sortie est toujours au format 16:9.", "Se utiliza en el modo Cámara y cuando Automático selecciona Cámara. La salida siempre es 16:9."),
        "Selected webcam" => ("Webcam sélectionnée", "Cámara web seleccionada"),
        "No webcam frame — choose a camera or refresh the device list." => ("Aucune image de webcam — choisissez une caméra ou actualisez la liste des appareils.", "No hay imagen de la cámara web: elige una cámara o actualiza la lista de dispositivos."),
        "Refresh camera devices" => ("Actualiser les caméras", "Actualizar cámaras"),
        "Crop webcam to 16:9" => ("Recadrer la webcam en 16:9", "Recortar la cámara web a 16:9"),
        "Crop non-16:9 cameras to fill the frame." => ("Recadrer les caméras qui ne sont pas en 16:9 afin de remplir l’image.", "Recortar las cámaras que no sean 16:9 para llenar el encuadre."),
        "No display selected" => ("Aucun écran sélectionné", "No hay ninguna pantalla seleccionada"),
        "Screen capture" => ("Capture d’écran", "Captura de pantalla"),
        "This display is used by Display mode and watched by Automatic mode for reference changes." => ("Cet écran est utilisé par le mode Écran et surveillé par le mode Automatique pour détecter les changements de référence.", "Esta pantalla se utiliza en el modo Pantalla y el modo Automático la vigila para detectar cambios en la referencia."),
        "Live screen" => ("Écran en direct", "Pantalla en directo"),
        "No screen frame — choose a display or use Recovery in Diagnostics." => ("Aucune image d’écran — choisissez un écran ou utilisez Récupération dans Diagnostic.", "No hay imagen de pantalla: elige una pantalla o usa Recuperación en Diagnóstico."),
        "Capture" => ("Capture", "Captura"),
        "Capture behavior" => ("Comportement de la capture", "Comportamiento de la captura"),
        "Include mouse cursor" => ("Inclure le pointeur de la souris", "Incluir el cursor del ratón"),
        "New references follow this choice; existing and imported references do not change." => ("Les nouvelles références suivent ce choix ; les références existantes et importées ne changent pas.", "Las referencias nuevas siguen esta opción; las existentes e importadas no cambian."),
        "Automatic discovery and recovery" => ("Découverte et récupération automatiques", "Detección y recuperación automáticas"),
        "Find reference display automatically" => ("Trouver automatiquement l’écran de référence", "Buscar automáticamente la pantalla de referencia"),
        "Recover black screen capture automatically" => ("Récupérer automatiquement une capture noire", "Recuperar automáticamente una captura en negro"),
        "On — Searches at launch, Settings open, reference changes, and every 30 seconds; confirms the same display twice." => ("Activé — Recherche au démarrage, à l’ouverture des Paramètres, lors des changements de référence et toutes les 30 secondes ; confirme deux fois le même écran.", "Activado: busca al iniciar, al abrir Ajustes, cuando cambia la referencia y cada 30 segundos; confirma dos veces la misma pantalla."),
        "Off — Choose a display manually or use Rescan displays." => ("Désactivé — Choisissez un écran manuellement ou utilisez Rechercher les écrans.", "Desactivado: elige una pantalla manualmente o usa Buscar pantallas."),
        "On — Checks the selected display every 30 seconds and restarts after two black results. Black content can trigger recovery." => ("Activé — Vérifie l’écran sélectionné toutes les 30 secondes et redémarre après deux résultats noirs. Un contenu noir peut déclencher la récupération.", "Activado: comprueba la pantalla seleccionada cada 30 segundos y reinicia tras dos resultados en negro. El contenido negro puede activar la recuperación."),
        "Off — Use Restart screen capture in Diagnostics." => ("Désactivé — Utilisez Redémarrer la capture d’écran dans Diagnostic.", "Desactivado: usa Reiniciar captura de pantalla en Diagnóstico."),

        // Matching.
        "Reference matching" => ("Correspondance de référence", "Coincidencia de referencia"),
        "Reference matches → Camera. Reference changes → Display. Without a usable reference, Automatic mode stays on Camera." => ("Référence identique → Caméra. Référence modifiée → Écran. Sans référence utilisable, le mode Automatique reste sur Caméra.", "La referencia coincide → Cámara. La referencia cambia → Pantalla. Sin una referencia válida, el modo Automático permanece en Cámara."),
        "Reference matches: Camera. Reference changes: Display. Without a usable reference, Automatic mode stays on Camera." => ("Référence identique : Caméra. Référence modifiée : Écran. Sans référence utilisable, le mode Automatique reste sur Caméra.", "La referencia coincide: Cámara. La referencia cambia: Pantalla. Sin una referencia válida, el modo Automático permanece en Cámara."),
        "Reference image" => ("Image de référence", "Imagen de referencia"),
        "No reference image — capture the current screen or import one." => ("Aucune image de référence — capturez l’écran actuel ou importez-en une.", "No hay imagen de referencia: captura la pantalla actual o importa una."),
        "Checks 4×/s · 5 matches or 3 mismatches · 0.5s fade" => ("4 vérifications/s · 5 correspondances ou 3 différences · fondu de 0,5 s", "4 comprobaciones/s · 5 coincidencias o 3 diferencias · fundido de 0,5 s"),
        "Capture screen" => ("Capturer l’écran", "Capturar pantalla"),
        "Import image…" => ("Importer une image…", "Importar imagen…"),
        "Match strictness" => ("Précision de la correspondance", "Precisión de coincidencia"),
        "Reset 98%" => ("Réinitialiser à 98 %", "Restablecer al 98 %"),
        "Very strict" => ("Très stricte", "Muy estricta"),
        "Balanced" => ("Équilibrée", "Equilibrada"),
        "Forgiving" => ("Tolérante", "Flexible"),
        "Very forgiving" => ("Très tolérante", "Muy flexible"),
        "Small visual changes can switch Automatic mode to Display." => ("De petits changements visuels peuvent faire basculer le mode Automatique vers Écran.", "Los pequeños cambios visuales pueden hacer que el modo Automático cambie a Pantalla."),
        "Minor rendering or cursor differences can still match the reference." => ("De légères différences de rendu ou de pointeur peuvent toujours correspondre à la référence.", "Las pequeñas diferencias de renderizado o del cursor aún pueden coincidir con la referencia."),
        "Larger differences may still count as the reference." => ("Des différences plus importantes peuvent toujours être considérées comme la référence.", "Las diferencias mayores aún pueden contar como referencia."),
        "Meaningful changes may still be treated as a match." => ("Des changements significatifs peuvent toujours être considérés comme une correspondance.", "Los cambios importantes aún pueden tratarse como coincidencias."),

        // Diagnostics.
        "Component health" => ("État des composants", "Estado de los componentes"),
        "Current pipeline state." => ("État actuel du pipeline.", "Estado actual del flujo."),
        "Virtual camera" => ("Caméra virtuelle", "Cámara virtual"),
        "Recovery" => ("Récupération", "Recuperación"),
        "Rescan finds the reference display. Restart buttons reconnect only the named component." => ("La recherche retrouve l’écran de référence. Les boutons de redémarrage reconnectent uniquement le composant indiqué.", "La búsqueda encuentra la pantalla de referencia. Los botones de reinicio solo vuelven a conectar el componente indicado."),
        "Rescan displays" => ("Rechercher les écrans", "Buscar pantallas"),
        "Restart webcam" => ("Redémarrer la webcam", "Reiniciar cámara web"),
        "Restart screen capture" => ("Redémarrer la capture d’écran", "Reiniciar captura de pantalla"),
        "Restart virtual camera" => ("Redémarrer la caméra virtuelle", "Reiniciar cámara virtual"),
        "Restart all" => ("Tout redémarrer", "Reiniciar todo"),
        "Technical details" => ("Détails techniques", "Detalles técnicos"),
        "Identifiers, formats, and timing used by the active pipeline." => ("Identifiants, formats et temporisation utilisés par le pipeline actif.", "Identificadores, formatos y tiempos utilizados por el flujo activo."),
        "Webcam device ID" => ("Identifiant de la webcam", "ID de la cámara web"),
        "Webcam format" => ("Format de la webcam", "Formato de la cámara web"),
        "Output pipeline" => ("Pipeline de sortie", "Flujo de salida"),
        "Transitions" => ("Transitions", "Transiciones"),
        "Detection timing" => ("Temporisation de la détection", "Frecuencia de detección"),
        "Storage and logs" => ("Stockage et journaux", "Almacenamiento y registros"),
        "Settings, references, and 14-day logs stay on this computer." => ("Les paramètres, les références et les journaux conservés 14 jours restent sur cet ordinateur.", "Los ajustes, las referencias y los registros de 14 días permanecen en este equipo."),
        "Configuration" => ("Configuration", "Configuración"),
        "Log directory" => ("Dossier des journaux", "Carpeta de registros"),
        "Diagnostic logs" => ("Journaux de diagnostic", "Registros de diagnóstico"),
        "Stored locally for troubleshooting." => ("Stockés localement pour le dépannage.", "Se guardan localmente para solucionar problemas."),
        "Logs are retained for 14 days." => ("Les journaux sont conservés pendant 14 jours.", "Los registros se conservan durante 14 días."),
        "Open log folder" => ("Ouvrir le dossier des journaux", "Abrir carpeta de registros"),
        "Export logs…" => ("Exporter les journaux…", "Exportar registros…"),
        "Clear logs…" => ("Effacer les journaux…", "Borrar registros…"),
        "Open folder" => ("Ouvrir le dossier", "Abrir carpeta"),
        "Export…" => ("Exporter…", "Exportar…"),
        "Clear…" => ("Effacer…", "Borrar…"),
        "The webcam needs attention. Choose or refresh the camera in Webcam, then restart it here if needed." => ("La webcam nécessite votre attention. Choisissez ou actualisez la caméra dans Webcam, puis redémarrez-la ici si nécessaire.", "La cámara web requiere atención. Elige o actualiza la cámara en Cámara web y reiníciala aquí si es necesario."),
        "Screen capture needs attention. Choose a display in Screen, then restart capture here if needed." => ("La capture d’écran nécessite votre attention. Choisissez un écran dans Écran, puis redémarrez la capture ici si nécessaire.", "La captura de pantalla requiere atención. Elige una pantalla en Pantalla y reinicia la captura aquí si es necesario."),
        "The virtual camera needs attention. Restart it here, then reselect StageSwap in the meeting app if necessary." => ("La caméra virtuelle nécessite votre attention. Redémarrez-la ici, puis sélectionnez de nouveau StageSwap dans l’application de réunion si nécessaire.", "La cámara virtual requiere atención. Reiníciala aquí y vuelve a seleccionar StageSwap en la aplicación de reuniones si es necesario."),
        "One or more components are still starting. Wait briefly before using a recovery action." => ("Un ou plusieurs composants sont encore en cours de démarrage. Patientez un instant avant d’utiliser une action de récupération.", "Uno o varios componentes todavía se están iniciando. Espera un momento antes de usar una acción de recuperación."),
        "The pipeline is ready, but Automatic mode needs a captured or imported reference." => ("Le pipeline est prêt, mais le mode Automatique nécessite une référence capturée ou importée.", "El flujo está listo, pero el modo Automático necesita una referencia capturada o importada."),
        "The components are ready; StageSwap is waiting for enough reference checks to decide." => ("Les composants sont prêts ; StageSwap attend suffisamment de vérifications de la référence pour prendre une décision.", "Los componentes están listos; StageSwap espera suficientes comprobaciones de la referencia para decidir."),
        "Everything is ready." => ("Tout est prêt.", "Todo está listo."),
        "Admin configuration" => ("Configuration administrateur", "Configuración de administrador"),

        // Dialogs.
        "Exit StageSwap?" => ("Quitter StageSwap ?", "¿Salir de StageSwap?"),
        "Clear diagnostic logs?" => ("Effacer les journaux de diagnostic ?", "¿Borrar los registros de diagnóstico?"),
        "Replace saved configuration?" => ("Remplacer la configuration enregistrée ?", "¿Sustituir la configuración guardada?"),
        "Load saved configuration?" => ("Charger la configuration enregistrée ?", "¿Cargar la configuración guardada?"),
        "Delete saved configuration?" => ("Supprimer la configuration enregistrée ?", "¿Eliminar la configuración guardada?"),
        "StageSwap will stop publishing. The virtual camera stays installed and shows the StageSwap off screen until the app starts again." => ("StageSwap cessera de diffuser. La caméra virtuelle reste installée et affiche l’écran d’arrêt de StageSwap jusqu’au prochain démarrage de l’application.", "StageSwap dejará de emitir. La cámara virtual seguirá instalada y mostrará la pantalla de StageSwap desactivado hasta que la aplicación vuelva a iniciarse."),
        "This permanently removes locally stored diagnostic logs. New logs will continue to be recorded." => ("Cette action supprime définitivement les journaux de diagnostic stockés localement. De nouveaux journaux continueront d’être enregistrés.", "Esto elimina de forma permanente los registros de diagnóstico guardados localmente. Se seguirán registrando nuevos datos."),
        "Replace the saved admin config with the setup currently shown in Settings?" => ("Remplacer la configuration administrateur enregistrée par celle actuellement affichée dans Paramètres ?", "¿Sustituir la configuración de administrador guardada por la que se muestra actualmente en Ajustes?"),
        "Replace the current settings and reference image with the saved admin config? Current session changes will be lost." => ("Remplacer les paramètres actuels et l’image de référence par la configuration administrateur enregistrée ? Les modifications de la session seront perdues.", "¿Sustituir los ajustes actuales y la imagen de referencia por la configuración de administrador guardada? Se perderán los cambios de la sesión."),
        "Auto-restore will turn off. Your current settings and reference image will stay unchanged." => ("La restauration automatique sera désactivée. Vos paramètres et votre image de référence actuels resteront inchangés.", "La restauración automática se desactivará. Los ajustes actuales y la imagen de referencia no cambiarán."),
        "Stay open" => ("Rester ouvert", "Mantener abierto"),
        "Exit StageSwap" => ("Quitter StageSwap", "Salir de StageSwap"),
        "Keep logs" => ("Conserver les journaux", "Conservar registros"),
        "Clear logs" => ("Effacer les journaux", "Borrar registros"),
        "Keep saved configuration" => ("Conserver la configuration enregistrée", "Conservar configuración guardada"),
        "Save current configuration" => ("Enregistrer la configuration actuelle", "Guardar configuración actual"),
        "Keep current config" => ("Conserver la configuration actuelle", "Conservar configuración actual"),
        "Load saved configuration" => ("Charger la configuration enregistrée", "Cargar configuración guardada"),
        "Delete saved configuration" => ("Supprimer la configuration enregistrée", "Eliminar configuración guardada"),
        "Keep a protected local copy of the current settings and reference image for managed setups." => ("Conservez une copie locale protégée des paramètres actuels et de l’image de référence pour les installations gérées.", "Conserva una copia local protegida de los ajustes actuales y la imagen de referencia para instalaciones administradas."),
        "No admin config is saved." => ("Aucune configuration administrateur n’est enregistrée.", "No hay ninguna configuración de administrador guardada."),
        "Settings and reference image saved in the admin config" => ("Paramètres et image de référence enregistrés dans la configuration administrateur", "Ajustes e imagen de referencia guardados en la configuración de administrador"),
        "Admin config saved without a reference image" => ("Configuration administrateur enregistrée sans image de référence", "Configuración de administrador guardada sin imagen de referencia"),
        "Auto-restore on launch" => ("Restaurer automatiquement au démarrage", "Restaurar automáticamente al iniciar"),
        "Replace session changes with this admin config whenever StageSwap starts." => ("Remplacer les modifications de la session par cette configuration administrateur à chaque démarrage de StageSwap.", "Sustituir los cambios de la sesión por esta configuración de administrador cada vez que se inicie StageSwap."),

        // Full-page setup guide.
        "Step {0} of {1}" => ("Étape {0} sur {1}", "Paso {0} de {1}"),
        "Set up later" => ("Configurer plus tard", "Configurar más tarde"),
        "Start StageSwap" => ("Démarrer StageSwap", "Iniciar StageSwap"),
        "StageSwap compares your screen with a reference image." => ("StageSwap compare votre écran à une image de référence.", "StageSwap compara tu pantalla con una imagen de referencia."),
        "Screen matches the reference → Zoom sees your webcam" => ("L’écran correspond à la référence → Zoom affiche votre webcam", "La pantalla coincide con la referencia → Zoom muestra tu cámara web"),
        "Screen does not match the reference → Zoom sees your screen" => ("L’écran ne correspond pas à la référence → Zoom affiche votre écran", "La pantalla no coincide con la referencia → Zoom muestra tu pantalla"),
        "Choose StageSwap as your camera in Zoom." => ("Choisissez StageSwap comme caméra dans Zoom.", "Elige StageSwap como cámara en Zoom."),
        "Choose your webcam" => ("Choisissez votre webcam", "Elige tu cámara web"),
        "Choose the webcam you want people in Zoom to see." => ("Choisissez la webcam que vous souhaitez montrer aux participants dans Zoom.", "Elige la cámara web que quieres que vean las personas en Zoom."),
        "Webcam preview" => ("Aperçu de la webcam", "Vista previa de la cámara web"),
        "No webcam found. Connect a webcam, then refresh the list." => ("Aucune webcam détectée. Connectez une webcam, puis actualisez la liste.", "No se encontró ninguna cámara web. Conecta una y actualiza la lista."),
        "This webcam is unavailable. Choose another one or refresh the list." => ("Cette webcam est indisponible. Choisissez-en une autre ou actualisez la liste.", "Esta cámara web no está disponible. Elige otra o actualiza la lista."),
        "Refresh webcams" => ("Actualiser les webcams", "Actualizar cámaras web"),
        "Choose your screen" => ("Choisissez votre écran", "Elige tu pantalla"),
        "Choose the screen you want StageSwap to show in Zoom." => ("Choisissez l’écran que StageSwap doit afficher dans Zoom.", "Elige la pantalla que quieres que StageSwap muestre en Zoom."),
        "Screen preview" => ("Aperçu de l’écran", "Vista previa de la pantalla"),
        "No screen found. Connect a screen, then rescan." => ("Aucun écran détecté. Connectez un écran, puis relancez la recherche.", "No se encontró ninguna pantalla. Conecta una y vuelve a buscar."),
        "This screen is unavailable. Choose another one or rescan." => ("Cet écran est indisponible. Choisissez-en un autre ou relancez la recherche.", "Esta pantalla no está disponible. Elige otra o vuelve a buscar."),
        "Set up automatic switching" => ("Configurez le basculement automatique", "Configura el cambio automático"),
        "Show your normal holding or idle picture on the screen you selected. Then capture it as the reference." => ("Affichez votre image d’attente habituelle sur l’écran sélectionné. Capturez-la ensuite comme référence.", "Muestra en la pantalla seleccionada tu imagen habitual de espera o inactividad. Después, captúrala como referencia."),
        "When the screen matches the reference, StageSwap shows your webcam. When the screen changes, StageSwap shows the screen." => ("Lorsque l’écran correspond à la référence, StageSwap affiche votre webcam. Lorsque l’écran change, StageSwap affiche l’écran.", "Cuando la pantalla coincide con la referencia, StageSwap muestra tu cámara web. Cuando la pantalla cambia, StageSwap muestra la pantalla."),
        "Reference preview" => ("Aperçu de la référence", "Vista previa de la referencia"),
        "No reference captured yet." => ("Aucune référence capturée pour le moment.", "Aún no se ha capturado ninguna referencia."),
        "Reference captured" => ("Référence capturée", "Referencia capturada"),
        "Capture again" => ("Capturer à nouveau", "Capturar de nuevo"),
        "StageSwap couldn’t capture the screen. Check the screen preview and try again." => ("StageSwap n’a pas pu capturer l’écran. Vérifiez l’aperçu de l’écran et réessayez.", "StageSwap no pudo capturar la pantalla. Comprueba la vista previa de la pantalla e inténtalo de nuevo."),
        "You’re ready" => ("Tout est prêt", "Todo listo"),
        "Before your Zoom call, choose StageSwap as your camera in Zoom." => ("Avant votre appel Zoom, choisissez StageSwap comme caméra dans Zoom.", "Antes de tu llamada de Zoom, elige StageSwap como cámara en Zoom."),
        "Webcam ready" => ("Webcam prête", "Cámara web lista"),
        "Webcam not selected" => ("Webcam non sélectionnée", "Cámara web no seleccionada"),
        "Screen ready" => ("Écran prêt", "Pantalla lista"),
        "Screen not selected" => ("Écran non sélectionné", "Pantalla no seleccionada"),
        "Reference ready" => ("Référence prête", "Referencia lista"),
        "Reference not captured" => ("Référence non capturée", "Referencia no capturada"),
        "Some setup is missing. StageSwap will start, but Automatic mode may not work as expected. You can finish setup later in Settings." => ("Certains éléments ne sont pas configurés. StageSwap va démarrer, mais le mode Automatique risque de ne pas fonctionner comme prévu. Vous pourrez terminer la configuration plus tard dans Paramètres.", "Faltan algunos elementos de la configuración. StageSwap se iniciará, pero es posible que el modo Automático no funcione como esperas. Puedes terminar la configuración más tarde en Ajustes."),
        "Setup guide" => ("Guide de configuration", "Guía de configuración"),
        "Learn how StageSwap works and choose your webcam, screen, and reference." => ("Découvrez le fonctionnement de StageSwap et choisissez votre webcam, votre écran et votre référence.", "Aprende cómo funciona StageSwap y elige la cámara web, la pantalla y la referencia."),
        "Open setup guide" => ("Ouvrir le guide de configuration", "Abrir guía de configuración"),

        // Tray and Windows notifications.
        "Open StageSwap" => ("Ouvrir StageSwap", "Abrir StageSwap"),
        "Webcam only" => ("Webcam uniquement", "Solo cámara web"),
        "Screen only" => ("Écran uniquement", "Solo pantalla"),
        "Exit" => ("Quitter", "Salir"),
        "StageSwap needs attention" => ("StageSwap nécessite votre attention", "StageSwap requiere atención"),
        "StageSwap installation failed" => ("Échec de l’installation de StageSwap", "Error al instalar StageSwap"),
        "StageSwap is already running" => ("StageSwap est déjà en cours d’exécution", "StageSwap ya está en ejecución"),
        "StageSwap could not start" => ("Impossible de démarrer StageSwap", "No se pudo iniciar StageSwap"),
        "StageSwap could not start its local control service" => ("StageSwap n’a pas pu démarrer son service de contrôle local", "StageSwap no pudo iniciar su servicio de control local"),
        "StageSwap deployment failed" => ("Échec du déploiement de StageSwap", "Error en el despliegue de StageSwap"),
        "Import reference image" => ("Importer une image de référence", "Importar imagen de referencia"),
        "Export diagnostic logs" => ("Exporter les journaux de diagnostic", "Exportar registros de diagnóstico"),
        "Image files" => ("Fichiers image", "Archivos de imagen"),
        "PNG files" => ("Fichiers PNG", "Archivos PNG"),
        "JPEG files" => ("Fichiers JPEG", "Archivos JPEG"),
        "BMP files" => ("Fichiers BMP", "Archivos BMP"),
        "JSON Lines files" => ("Fichiers JSON Lines", "Archivos JSON Lines"),
        "All files" => ("Tous les fichiers", "Todos los archivos"),

        // Portable install and update prompts.
        "Replace the installed StageSwap?" => ("Remplacer la version installée de StageSwap ?", "¿Sustituir la versión instalada de StageSwap?"),
        "Update installed StageSwap" => ("Mettre à jour StageSwap", "Actualizar StageSwap instalado"),
        "Open installed StageSwap" => ("Ouvrir StageSwap installé", "Abrir StageSwap instalado"),
        "Install StageSwap for this user?" => ("Installer StageSwap pour cet utilisateur ?", "¿Instalar StageSwap para este usuario?"),
        "Installation keeps the app at a stable per-user path and creates Start Menu and Desktop shortcuts. The virtual-camera component still requires administrator approval." => ("L’installation conserve l’application dans un emplacement stable propre à l’utilisateur et crée des raccourcis dans le menu Démarrer et sur le Bureau. Le composant de caméra virtuelle nécessite toujours l’autorisation d’un administrateur.", "La instalación mantiene la aplicación en una ruta estable para el usuario y crea accesos directos en el menú Inicio y en el Escritorio. El componente de cámara virtual sigue necesitando aprobación de administrador."),
        "Install StageSwap\nRecommended for startup and upgrades" => ("Installer StageSwap\nRecommandé pour le démarrage et les mises à jour", "Instalar StageSwap\nRecomendado para el inicio y las actualizaciones"),
        "Run once\nDo not copy this executable" => ("Exécuter une fois\nNe pas copier cet exécutable", "Ejecutar una vez\nNo copiar este ejecutable"),
        "This replaces the installed app with an older version." => ("Cette opération remplace l’application installée par une version antérieure.", "Esto sustituye la aplicación instalada por una versión anterior."),
        "This replaces it with a different build of the same version." => ("Cette opération la remplace par une autre compilation de la même version.", "Esto la sustituye por otra compilación de la misma versión."),
        "The running app will close gracefully before replacement." => ("L’application en cours d’exécution se fermera correctement avant son remplacement.", "La aplicación en ejecución se cerrará correctamente antes de sustituirse.")
        ,"Installed version: {0}\nCandidate version: {1}\n\n{2}" => ("Version installée : {0}\nVersion candidate : {1}\n\n{2}", "Versión instalada: {0}\nVersión candidata: {1}\n\n{2}")
    }))
}

pub fn format_text(locale: Locale, source: &str, arguments: &[&str]) -> String {
    let mut result = text(locale, source).into_owned();
    for (index, value) in arguments.iter().enumerate() {
        result = result.replace(&format!("{{{index}}}"), value);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_tags_accept_language_families_and_normalize() {
        assert_eq!(Locale::from_tag("fr-CA"), Some(Locale::French));
        assert_eq!(Locale::from_tag("es_MX"), Some(Locale::Spanish));
        assert_eq!(Locale::from_tag("EN-gb"), Some(Locale::English));
        assert_eq!(Locale::from_tag("de-DE"), None);
        assert_eq!(Locale::Spanish.tag(), "es");
    }

    #[test]
    fn saved_locale_wins_then_system_then_english() {
        assert_eq!(
            Locale::resolve(Some("fr-FR"), Some("es-ES")),
            Locale::French
        );
        assert_eq!(Locale::resolve(None, Some("es-MX")), Locale::Spanish);
        assert_eq!(
            Locale::resolve(Some("unknown"), Some("fr-CA")),
            Locale::French
        );
        assert_eq!(Locale::resolve(None, Some("de-DE")), Locale::English);
    }

    #[test]
    fn known_text_is_translated_and_unknown_text_is_preserved() {
        assert_eq!(text(Locale::French, "Settings"), "Paramètres");
        assert_eq!(text(Locale::Spanish, "Settings"), "Ajustes");
        assert_eq!(
            text(Locale::French, "Device supplied name"),
            "Device supplied name"
        );
    }

    #[test]
    fn formatted_text_reorders_safe_placeholders() {
        assert_eq!(
            format_text(Locale::French, "{0} — {1}", &["98 %", "Équilibrée"]),
            "98 % — Équilibrée"
        );
    }

    #[test]
    fn setup_guide_copy_uses_the_approved_localizations() {
        assert_eq!(
            text(Locale::French, "Set up automatic switching"),
            "Configurez le basculement automatique"
        );
        assert_eq!(
            text(Locale::Spanish, "Set up automatic switching"),
            "Configura el cambio automático"
        );
        assert_eq!(
            text(Locale::French, "Open setup guide"),
            "Ouvrir le guide de configuration"
        );
        assert_eq!(text(Locale::Spanish, "You’re ready"), "Todo listo");
        assert_eq!(
            format_text(Locale::French, "Step {0} of {1}", &["4", "5"]),
            "Étape 4 sur 5"
        );
        assert_eq!(
            format_text(Locale::Spanish, "Step {0} of {1}", &["4", "5"]),
            "Paso 4 de 5"
        );
    }
}
